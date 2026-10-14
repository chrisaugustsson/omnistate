// `make chrome-boxes FILE=<design.json>`: exports a design as HTML with the CSS
// of docs/formats/layout.md (layout-export.mjs), renders it in headless Chrome
// at a device pixel ratio of 2, and prints the box of every node as one JSON
// object on standard output: `{"<node id>": {"x": 0, "y": 0, "width": 60,
// "height": 30}}`, in page coordinates, as `getBoundingClientRect` gives them.
// Everything else it says goes to standard error.
//
// Chrome is driven over the DevTools protocol with Node's own WebSocket, so
// there is nothing to install: Node 22 or later and Chrome. `CHROME` names the
// Chrome binary when it is not at its usual place. The expected boxes of the
// layout tests were measured with Chrome 155.
import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { boxesOf, bundledFonts, exportHtml } from './layout-export.mjs';

// The device pixel ratio that Chrome rounds line heights and baselines at, as
// the layout rules of docs/formats/layout.md assume.
const DEVICE_PIXEL_RATIO = 2;

// The Chrome that the expected boxes were measured with.
const MEASURED_WITH_CHROME = 155;

// How long Chrome may take to start, to render the design and answer, and to
// close when it is asked to.
const STARTUP_TIMEOUT_MS = 60_000;
const RENDER_TIMEOUT_MS = 170_000;
const CLOSE_TIMEOUT_MS = 5_000;

const CHROME_NAMES = ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'];
const CHROME_MACOS = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

function findChrome() {
  const named = process.env.CHROME;
  if (named) return named;
  if (existsSync(CHROME_MACOS)) return CHROME_MACOS;
  for (const directory of (process.env.PATH ?? '').split(path.delimiter)) {
    for (const name of CHROME_NAMES) {
      const candidate = path.join(directory, name);
      if (directory && existsSync(candidate)) return candidate;
    }
  }
  throw new Error('Chrome is not installed where it usually is: name its binary in the CHROME variable');
}

// Starts headless Chrome with a fresh profile, and resolves with the process
// and the address of its DevTools endpoint.
function startChrome(chrome, profile) {
  const child = spawn(
    chrome,
    [
      '--headless=new',
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      `--force-device-scale-factor=${DEVICE_PIXEL_RATIO}`,
      '--window-size=1440,900',
      '--hide-scrollbars',
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-extensions',
      '--disable-background-networking',
      '--disable-component-update',
      '--disable-sync',
      '--mute-audio',
      // No password prompts from the macOS keychain for a throwaway profile.
      '--password-store=basic',
      '--use-mock-keychain',
      'about:blank',
    ],
    { stdio: ['ignore', 'ignore', 'pipe'] },
  );
  return new Promise((resolve, reject) => {
    let log = '';
    const timer = setTimeout(
      () => reject(new Error(`Chrome did not start within ${STARTUP_TIMEOUT_MS / 1000} s:\n${log}`)),
      STARTUP_TIMEOUT_MS,
    );
    child.stderr.setEncoding('utf8');
    child.stderr.on('data', (text) => {
      log += text;
      const endpoint = /DevTools listening on (ws:\/\/\S+)/.exec(log);
      if (endpoint) {
        clearTimeout(timer);
        resolve({ child, endpoint: endpoint[1] });
      }
    });
    child.on('error', (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on('exit', (code, signal) => {
      clearTimeout(timer);
      reject(new Error(`Chrome exited (${code ?? signal}) before it was ready:\n${log}`));
    });
  });
}

// The few DevTools commands that this tool needs, over one WebSocket.
class DevTools {
  static connect(endpoint) {
    return new Promise((resolve, reject) => {
      const socket = new WebSocket(endpoint);
      socket.addEventListener('open', () => resolve(new DevTools(socket)));
      socket.addEventListener('error', () => reject(new Error(`cannot connect to Chrome at ${endpoint}`)));
    });
  }

  constructor(socket) {
    this.socket = socket;
    this.nextId = 1;
    this.pending = new Map();
    this.waiting = [];
    socket.addEventListener('message', ({ data }) => {
      const message = JSON.parse(data);
      if (message.id !== undefined) {
        const { resolve, reject } = this.pending.get(message.id);
        this.pending.delete(message.id);
        if (message.error) reject(new Error(`${message.error.message} (${JSON.stringify(message.error)})`));
        else resolve(message.result);
      } else {
        this.waiting = this.waiting.filter((waiter) => !waiter(message));
      }
    });
    socket.addEventListener('close', () => {
      for (const { reject } of this.pending.values()) reject(new Error('Chrome closed the connection'));
      this.pending.clear();
    });
  }

  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.socket.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }

  // Resolves when Chrome sends the event `method` for `sessionId`.
  event(method, sessionId) {
    return new Promise((resolve) => {
      this.waiting.push((message) => {
        const matches = message.method === method && message.sessionId === sessionId;
        if (matches) resolve(message.params);
        return matches;
      });
    });
  }

  close() {
    this.socket.close();
  }
}

// What the page does once it has loaded: waits for the bundled fonts, so that
// no text is measured in a fallback font, and reads the rectangle of every node.
const MEASURE = `(async () => {
  await Promise.all([
    document.fonts.load('16px Inter'),
    document.fonts.load('italic 16px Inter'),
  ]);
  await document.fonts.ready;
  const unloaded = [...document.fonts].filter((face) => face.status !== 'loaded');
  if (unloaded.length > 0) {
    throw new Error('the bundled fonts did not load: ' + unloaded.map((face) => face.family + ' ' + face.status));
  }
  const rects = {};
  for (const element of document.querySelectorAll('[data-node]')) {
    const rect = element.getBoundingClientRect();
    rects[element.dataset.node] = [rect.x + scrollX, rect.y + scrollY, rect.width, rect.height];
  }
  return { devicePixelRatio, version: navigator.userAgent, rects };
})()`;

async function measure(chrome, html) {
  const directory = await mkdtemp(path.join(tmpdir(), 'omni-chrome-boxes-'));
  let running;
  try {
    const page = path.join(directory, 'design.html');
    await writeFile(page, html);
    running = await startChrome(chrome, path.join(directory, 'profile'));
    const devtools = await DevTools.connect(running.endpoint);
    const render = (async () => {
      const { targetId } = await devtools.send('Target.createTarget', { url: 'about:blank' });
      const { sessionId } = await devtools.send('Target.attachToTarget', { targetId, flatten: true });
      await devtools.send('Page.enable', {}, sessionId);
      const loaded = devtools.event('Page.loadEventFired', sessionId);
      await devtools.send('Page.navigate', { url: pathToFileURL(page).href }, sessionId);
      await loaded;
      const { result, exceptionDetails } = await devtools.send(
        'Runtime.evaluate',
        { expression: MEASURE, awaitPromise: true, returnByValue: true },
        sessionId,
      );
      if (exceptionDetails) {
        throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
      }
      return result.value;
    })();
    const timeout = new Promise((_, reject) => {
      const giveUp = () =>
        reject(new Error(`Chrome did not render the design within ${RENDER_TIMEOUT_MS / 1000} s`));
      setTimeout(giveUp, RENDER_TIMEOUT_MS).unref();
    });
    try {
      return await Promise.race([render, timeout]);
    } finally {
      await stopChrome(running.child, devtools);
    }
  } finally {
    if (running) await stopChrome(running.child);
    await rm(directory, { recursive: true, force: true });
  }
}

// Closes Chrome, asking it to when there is still a connection, and kills it if
// it does not go, then waits until it is gone, so that nothing is left running
// that holds the profile.
async function stopChrome(child, devtools = undefined) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  const gone = new Promise((resolve) => child.once('exit', resolve));
  if (devtools) {
    devtools.send('Browser.close').catch(() => {});
    const killer = setTimeout(() => child.kill('SIGKILL'), CLOSE_TIMEOUT_MS);
    await gone;
    clearTimeout(killer);
    devtools.close();
  } else {
    child.kill('SIGKILL');
    await gone;
  }
}

async function main() {
  const [file] = process.argv.slice(2);
  if (!file) {
    throw new Error('usage: make chrome-boxes FILE=<design.json>');
  }
  if (typeof WebSocket === 'undefined') {
    throw new Error(`Node ${process.versions.node} has no WebSocket: use Node 22 or later`);
  }
  const design = JSON.parse(readFileSync(file, 'utf8'));
  const html = exportHtml(design, bundledFonts());
  const { devicePixelRatio, version, rects } = await measure(findChrome(), html);
  if (devicePixelRatio !== DEVICE_PIXEL_RATIO) {
    throw new Error(
      `Chrome rendered at a device pixel ratio of ${devicePixelRatio}, not ${DEVICE_PIXEL_RATIO}`,
    );
  }
  const major = Number(/Chrome\/(\d+)/.exec(version)?.[1]);
  if (major !== MEASURED_WITH_CHROME) {
    console.error(
      `note: the expected boxes of the layout tests were measured with Chrome ${MEASURED_WITH_CHROME}, and this is ${version}`,
    );
  }
  process.stdout.write(`${JSON.stringify(boxesOf(design, rects))}\n`);
}

main().catch((error) => {
  console.error(`chrome-boxes: ${error.message}`);
  process.exitCode = 1;
});
