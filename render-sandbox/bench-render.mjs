// `make bench-render`: renderer gate A (US-008). Runs the stress file in headed
// Chrome, through Playwright, at 1440x900 and a device pixel ratio of 2, and
// takes each measurement of bench.html (main-bench.js) 5 times from a cold page
// load: a new Chrome with an empty profile for every one. The medians go to the
// Gate A section of docs/renderer-gate.md, which is written on the machine it
// ran on. The exit status is 0 only if every limit held.
//
// The run is stopped, and nothing written, if the page's WebGL renderer string
// shows software rendering, such as SwiftShader: that would measure the CPU.
//
// `make bench-render` writes the inputs (build/omni_render.wasm and
// build/stress-file.json) and installs Playwright with npm first. It drives the
// Chrome that is installed on the machine (`channel: 'chrome'`), so no browser is
// downloaded. `CHROME` names another Chrome binary.
import { createReadStream, existsSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { assertHardwareRenderer, config, evaluate, renderReport } from './bench-render-lib.mjs';

const SANDBOX = path.dirname(fileURLToPath(import.meta.url));
const REPORT = path.join(SANDBOX, '..', 'docs', 'renderer-gate.md');
// Where the Gate A section starts in docs/renderer-gate.md, and the heading of
// the document if there is none yet. A later gate adds a section after it, so
// only the Gate A section is replaced.
const SECTION = '## Gate A';
const TITLE = '# Renderer gate\n\nThe pass/fail tests of the renderer (Q35). Each section is written by the script of its gate.\n';
const MEASUREMENTS = ['board', 'file', 'frames'];
// How long a page may take to measure, the zoom and pan path included.
const MEASURE_TIMEOUT_MS = 120_000;

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
};

// Serves render-sandbox/ on a free local port, so that the page can load its
// module, its WebAssembly and the stress file.
function serve() {
  const server = http.createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    const file = path.join(SANDBOX, pathname);
    if (!file.startsWith(SANDBOX + path.sep) || !existsSync(file)) {
      response.writeHead(404).end('not found');
      return;
    }
    response.writeHead(200, {
      'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream',
      'cache-control': 'no-store',
    });
    createReadStream(file).pipe(response);
  });
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve({ server, port: server.address().port }));
  });
}

// The unmasked WebGL renderer string of the page's own context, the canvas that
// the page drew on.
function rendererOf(page) {
  return page.evaluate(() => {
    const gl = document.querySelector('canvas').getContext('webgl2');
    const info = gl.getExtension('WEBGL_debug_renderer_info');
    return gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
  });
}

// One measurement from a cold page load: a new Chrome, a new page, nothing
// cached. Returns the result of the page, and the renderer it drew with.
async function measure(base, kind) {
  const browser = await chromium.launch({
    channel: 'chrome',
    executablePath: process.env.CHROME || undefined,
    headless: false,
  });
  try {
    const context = await browser.newContext({
      viewport: config.viewport,
      deviceScaleFactor: config.deviceScaleFactor,
    });
    const page = await context.newPage();
    page.on('pageerror', (error) => console.error(`  page error: ${error.message}`));
    await page.goto(`${base}/bench.html?measure=${kind}`);
    await page.waitForFunction(() => window.benchResult, null, { timeout: MEASURE_TIMEOUT_MS });
    const result = await page.evaluate(() => window.benchResult);
    if (result.error) throw new Error(`the ${kind} measurement failed in the page: ${result.error}`);
    const renderer = await rendererOf(page);
    assertHardwareRenderer(renderer);
    const { width, height } = config.viewport;
    if (result.devicePixelRatio !== config.deviceScaleFactor || result.viewport.join('x') !== `${width}x${height}`) {
      throw new Error(
        `the page ran at ${result.viewport.join('x')} and a pixel ratio of ${result.devicePixelRatio}, not ${width}x${height} at ${config.deviceScaleFactor}`,
      );
    }
    return { result, renderer, version: browser.version() };
  } finally {
    await browser.close();
  }
}

function machineName() {
  return `${os.cpus()[0].model}, ${Math.round(os.totalmem() / 2 ** 30)} GB memory, ${os.type()} ${os.release()} (${os.arch()})`;
}

// The report with the Gate A section replaced by `section`, or added after the
// sections that are there.
async function spliced(section) {
  const old = existsSync(REPORT) ? await readFile(REPORT, 'utf8') : TITLE;
  const start = old.indexOf(SECTION);
  if (start < 0) return `${old.trimEnd()}\n\n${section}`;
  const next = old.indexOf('\n## ', start + SECTION.length);
  return `${old.slice(0, start)}${section}${next < 0 ? '' : `\n${old.slice(next + 1)}`}`;
}

const { server, port } = await serve();
const base = `http://127.0.0.1:${port}`;
try {
  const runs = [];
  let renderer = '';
  let version = '';
  for (let run = 1; run <= config.runs; run += 1) {
    const values = {};
    for (const kind of MEASUREMENTS) {
      const measured = await measure(base, kind);
      Object.assign(values, measured.result);
      renderer = measured.renderer;
      version = measured.version;
    }
    console.log(
      `run ${run}: board ${values.boardMs.toFixed(1)} ms, file ${values.fileMs.toFixed(1)} ms, frame p95 ${values.frameP95Ms.toFixed(1)} ms`,
    );
    runs.push({ boardMs: values.boardMs, fileMs: values.fileMs, frameP95Ms: values.frameP95Ms });
  }

  const date = new Date().toISOString().slice(0, 10);
  const report = renderReport({ machine: `${machineName()}, Chrome ${version}`, renderer, runs, date });
  await writeFile(REPORT, await spliced(report));
  const result = evaluate(runs);
  for (const check of result.checks) {
    console.log(`${check.name}: median ${check.value.toFixed(1)} (limit ${check.limit}) ${check.pass ? 'PASS' : 'FAIL'}`);
  }
  console.log(`wrote docs/renderer-gate.md: the gate ${result.pass ? 'PASSED' : 'FAILED'}`);
  process.exitCode = result.pass ? 0 : 1;
} finally {
  server.close();
}
