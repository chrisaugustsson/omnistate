// `make bench-ordrar` (US-117): how Ordrar (fixtures/penpot/tirebud-ordrar.penpot,
// 797 boards) loads and moves in the real web app. It starts a stack of its own
// (release API, production web app, a Compose project named omnistate-bench with
// fresh volumes), imports Ordrar into a new project with `omni import penpot`,
// and then, five times, each from a restarted API container and a new headed
// Chrome with an empty cache (1440x900, device pixel ratio 2):
//
// - opens the design at /docs/:doc and times, from the navigation, the first
//   complete frame that shows the first board of the design's manifest in full:
//   read, all of it in view at full detail, with its text and images (the cold
//   time to first board); a real click on it must then select a node;
// - opens the page with the most boards and plays a scripted 10 s path over it:
//   from the opening board out to the viewer's minimum zoom, which on this page
//   shows about 350 of its 769 boards and not all of them, across the whole page
//   and back in on the opening board, timing every frame from the end of the
//   one before to `gl.finish()`;
// - samples the API container's memory as `docker stats` reports it, from before
//   the navigation to the end of the path, and keeps the highest sample.
//
// It prints the medians of the five runs and writes docs/bench-ordrar.md: the
// baseline numbers and a profile of where the time goes. The page side is
// bench-ordrar-page.js, the numbers are bench-ordrar-lib.mjs. The browser is
// the Chrome that is installed on the machine (`CHROME` names another binary),
// which must have a GPU: a software WebGL renderer stops the run.
//
// Environment: `OMNI_BENCH_PORT` (default 8083) is the port of the bench stack;
// `OMNI_BENCH_KEEP=1` leaves the stack running afterwards.
import { spawn, spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { assertHardwareRenderer } from './bench-render-lib.mjs';
import { config, evaluate, frameProfile, frameStats, memUsageMiB, renderReport, splitLoad } from './bench-ordrar-lib.mjs';

const SANDBOX = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.join(SANDBOX, '..');
const REPORT = path.join(ROOT, 'docs', 'bench-ordrar.md');
const FIXTURE = path.join(ROOT, 'fixtures', 'penpot', 'tirebud-ordrar.penpot');
const OMNI = path.join(ROOT, 'target', 'release', 'omni');
const PROJECT = 'omnistate-bench';
const PORT = process.env.OMNI_BENCH_PORT || '8083';
const BASE = `http://127.0.0.1:${PORT}`;
const EMAIL = 'bench@example.com';
const AS = ['--as', 'bench/importer'];
const TITLE = '# Ordrar benchmark\n';
// The part of the report that a person writes; a run keeps it.
const NOTES = '## Reading the profile';

function run(command, args, { input, env, check = true } = {}) {
  const result = spawnSync(command, args, {
    cwd: ROOT,
    input,
    env: { ...process.env, OMNI_PORT: PORT, ...env },
    encoding: 'utf8',
    maxBuffer: 1 << 28,
  });
  if (check && result.status !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed (${result.status}): ${(result.stderr || result.stdout).trim().slice(-600)}`);
  }
  return result;
}
const compose = (args, options) => run('docker', ['compose', '-p', PROJECT, ...args], options);

async function waitForApi() {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(`${BASE}/api/health`);
      if (response.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error('the API did not come back after its restart');
}

// A browser session of the human, which the web app needs to open a design.
async function login(password) {
  const response = await fetch(`${BASE}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ email: EMAIL, password }),
  });
  if (!response.ok) throw new Error(`the bench login failed: ${response.status}`);
  const cookie = response.headers.getSetCookie().find((value) => value.startsWith('omni_session='));
  return cookie.split(';')[0].slice('omni_session='.length);
}

const size = (value) => (typeof value === 'number' ? value : (value?.fixed ?? 0));
const box = (board) => ({ x: board.x ?? 0, y: board.y ?? 0, width: size(board.width), height: size(board.height) });

// The page with the most boards of the design's outline, its bounds and its
// first board, which is what the scripted path is made of.
function pathOf(outline) {
  const page = outline.reduce((most, candidate) => (candidate.children.length > most.children.length ? candidate : most));
  const boards = page.children.map(box);
  const left = Math.min(...boards.map((b) => b.x));
  const top = Math.min(...boards.map((b) => b.y));
  const right = Math.max(...boards.map((b) => b.x + b.width));
  const bottom = Math.max(...boards.map((b) => b.y + b.height));
  return {
    pageName: page.name,
    boards: page.children.length,
    cfg: { bounds: { x: left, y: top, width: right - left, height: bottom - top }, opening: boards[0], ms: config.pathMs },
  };
}

// Serves the renderer's module with the bench's hook on it, and nothing else of
// the app changed.
async function hookRenderer(context) {
  await context.route('**/render/omni_render.js', async (route) => {
    const response = await route.fetch();
    const text = await response.text();
    const marker = 'export default createOmniRender;';
    if (!text.includes(marker)) throw new Error('the renderer module has no default export that the bench can hook');
    const hooked = text.replace(
      marker,
      'export default async (...args) => { const module = await createOmniRender(...args); window.__benchHook(module); return module; };',
    );
    await route.fulfill({ response, body: hooked });
  });
}

// Follows the API container's memory with `docker stats`, which reports it about
// once a second. `ready` resolves with the first sample and `stop` gives the peak.
function sampleMemory() {
  const container = compose(['ps', '--quiet', 'api']).stdout.trim();
  const stats = spawn('docker', ['stats', '--format', '{{.MemUsage}}', container], { stdio: ['ignore', 'pipe', 'ignore'] });
  let peak = 0;
  let sampled;
  const ready = new Promise((resolve) => {
    sampled = resolve;
  });
  stats.stdout.setEncoding('utf8').on('data', (text) => {
    for (const line of text.split('\n')) {
      const usage = memUsageMiB(line);
      if (usage === null) continue;
      peak = Math.max(peak, usage);
      sampled();
    }
  });
  return {
    ready,
    stop: () => {
      stats.kill();
      return peak;
    },
  };
}

// A real click on the middle of the first board, with Cmd held for the deepest
// node there: the viewer must put the node in the address bar, with that board.
async function selectOnFirstBoard(page, board, firstBoardId) {
  const canvas = await page.evaluate(() => {
    const { left, top } = window.__bench.canvas.getBoundingClientRect();
    return { left, top };
  });
  await page.mouse.click(canvas.left + board.x + board.width / 2, canvas.top + board.y + board.height / 2, { modifiers: ['Meta'] });
  await page.waitForFunction(
    (id) => {
      const { searchParams } = new URL(location.href);
      return searchParams.get('board') === id && searchParams.has('node');
    },
    firstBoardId,
    { timeout: 10_000 },
  );
}

// Waits until the viewer has not loaded a page into the renderer for `quietMs`
// and has loaded more than `after` pages.
function settled(page, after, quietMs) {
  return page.waitForFunction(
    ([count, quiet]) => (window.__bench.loads ?? 0) > count && performance.now() - window.__bench.lastLoadEnd > quiet,
    [after, quietMs],
    { timeout: 120_000, polling: 100 },
  );
}

async function measureRun({ password, doc, workspace, projectId, view, firstBoardId, index }) {
  compose(['restart', 'api']);
  await waitForApi();
  // Peak memory counts from here; the import and the restart are before it.
  const memory = sampleMemory();
  await memory.ready;
  const token = await login(password);

  const browser = await chromium.launch({ channel: 'chrome', executablePath: process.env.CHROME || undefined, headless: false });
  try {
    const context = await browser.newContext({ viewport: config.viewport, deviceScaleFactor: config.deviceScaleFactor });
    await context.addCookies([{ name: 'omni_session', value: token, url: BASE, httpOnly: true, secure: true, sameSite: 'Lax' }]);
    await context.addInitScript({ content: `performance.setResourceTimingBufferSize(20000);\n${await readFile(path.join(SANDBOX, 'bench-ordrar-page.js'), 'utf8')}\nwindow.__bench.firstBoardId = ${JSON.stringify(firstBoardId)};` });
    await hookRenderer(context);
    const page = await context.newPage();
    page.on('pageerror', (error) => console.error(`  page error: ${error.message}`));
    page.on('console', (message) => {
      if (message.type() === 'error') console.error(`  console error: ${message.text().slice(0, 200)}`);
    });

    await page.goto(`${BASE}/w/${workspace}/projects/${projectId}/docs/${doc}`, { waitUntil: 'commit' });
    await page.waitForFunction(() => window.__bench?.firstBoard !== null && window.__bench?.firstBoard !== undefined, null, {
      timeout: 120_000,
      polling: 50,
    });
    const first = await page.evaluate(() => ({
      ms: window.__bench.firstBoard,
      info: window.__bench.firstBoardInfo,
      spans: window.__bench.spans,
    }));
    const renderer = await page.evaluate(() => {
      const gl = window.__bench.canvas.getContext('webgl2');
      const info = gl.getExtension('WEBGL_debug_renderer_info');
      return gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
    });
    assertHardwareRenderer(renderer);
    await selectOnFirstBoard(page, first.info.firstBoard, firstBoardId);

    // The big page, which opens on its first board.
    const loaded = await page.evaluate(() => window.__bench.loads ?? 0);
    await page.getByRole('button', { name: view.pageName, exact: true }).click();
    await settled(page, loaded, 1500);

    const played = await page.evaluate((cfg) => window.__bench.runPath(cfg), view.cfg);
    const peakMemoryMiB = memory.stop();
    const requests = await page.evaluate(() => {
      const entries = performance.getEntriesByType('resource').map((entry) => ({
        name: new URL(entry.name).pathname.replace(/[0-9a-f]{8}-[0-9a-f-]{27}/g, ':id'),
        start: entry.startTime,
        end: entry.responseEnd,
        bytes: entry.transferSize,
      }));
      const [navigation] = performance.getEntriesByType('navigation');
      return [{ name: 'document', start: 0, end: navigation.responseEnd, bytes: navigation.transferSize }, ...entries];
    });

    const frames = played.frames;
    const longTasks = await page.evaluate(() => window.__bench.longTasks);
    const cold = splitLoad(
      first.ms,
      [...first.spans, ...longTasks],
      requests.map((request) => [request.start, request.end]),
    );
    // The requests that began before the first board, longest first.
    const slowest = requests
      .filter((request) => request.start < first.ms)
      .sort((a, b) => b.end - b.start - (a.end - a.start))
      .slice(0, 6);
    const result = {
      firstBoardMs: first.ms,
      firstBoardInfo: first.info,
      cold,
      slowest,
      frames: frameStats(frames, played.ticks),
      frameProfile: frameProfile(frames),
      calls: Object.entries(played.calls)
        .map(([name, call]) => ({ name, ...call }))
        .sort((a, b) => b.ms - a.ms),
      peakMemoryMiB,
      renderer,
      version: browser.version(),
    };
    console.log(
      `run ${index}: first board ${result.firstBoardMs.toFixed(1)} ms, frames ${result.frames.count} (median ${result.frames.medianMs.toFixed(1)} ms, p95 ${result.frames.p95Ms.toFixed(1)} ms), API peak ${result.peakMemoryMiB.toFixed(0)} MiB`,
    );
    return result;
  } finally {
    memory.stop();
    await browser.close();
  }
}

function machineName() {
  return `${os.cpus()[0].model}, ${Math.round(os.totalmem() / 2 ** 30)} GB memory, ${os.type()} ${os.release()} (${os.arch()})`;
}

// The report: the title, the part this script writes, and the notes a person
// wrote before, if the report is there already.
async function spliced(generated) {
  const old = existsSync(REPORT) ? await readFile(REPORT, 'utf8') : '';
  const start = old.indexOf(NOTES);
  const notes = start < 0 ? '' : `\n${old.slice(start)}`;
  return `${TITLE}\n${generated.trimEnd()}\n${notes}`;
}

async function main() {
  if (!existsSync(FIXTURE)) throw new Error(`there is no ${path.relative(ROOT, FIXTURE)}`);
  const password = randomBytes(12).toString('hex');
  const home = await mkdtemp(path.join(os.tmpdir(), 'omni-bench-ordrar-'));
  const omni = (args, options = {}) => run(OMNI, args, { ...options, env: { HOME: home } });
  try {
    console.log('starting a fresh stack (release API, production web app)...');
    compose(['down', '--volumes', '--remove-orphans']);
    compose(['up', '--build', '--wait', '--detach', 'proxy']);
    compose(['exec', '-T', 'api', 'omni-api', 'setup', '--email', EMAIL, '--workspace', 'Bench', '--password-stdin'], { input: `${password}\n` });
    omni(['login', '--server', BASE], { input: `${EMAIL}\n${password}\n` });
    omni(['project', 'create', 'Ordrar', ...AS]);

    console.log('importing Ordrar...');
    const started = performance.now();
    const imported = JSON.parse(omni(['import', 'penpot', FIXTURE, '--project', 'Ordrar', '--json', ...AS]).stdout);
    const importMs = performance.now() - started;
    const doc = imported.design_doc_id;

    const token = await login(password);
    const headers = { cookie: `omni_session=${token}` };
    const { workspaces } = await (await fetch(`${BASE}/api/v1/auth/workspaces`, { headers })).json();
    const workspace = workspaces[0].id;
    const { projects } = await (await fetch(`${BASE}/api/v1/w/${workspace}/projects`, { headers })).json();
    const projectId = projects.find((project) => project.name === 'Ordrar').id;
    const outline = (await (await fetch(`${BASE}/api/v1/w/${workspace}/projects/${projectId}/docs/${doc}/html?view=outline`, { headers })).json());
    const boards = outline.boards.length;
    const view = pathOf(outline.outline);
    const firstBoardId = outline.boards[0].id;
    console.log(`${boards} boards on ${outline.outline.length} pages; the path runs on the page ${view.pageName} (${view.boards} boards)`);

    const runs = [];
    let renderer = '';
    let version = '';
    for (let index = 1; index <= config.runs; index += 1) {
      const result = await measureRun({ password, doc, workspace, projectId, view, firstBoardId, index });
      runs.push(result);
      renderer = result.renderer;
      version = result.version;
    }

    const result = evaluate(runs);
    const date = new Date().toISOString().slice(0, 10);
    const report = renderReport({
      machine: `${machineName()}, Chrome ${version}`,
      renderer,
      apiImage: 'release build of the API (docker/api.Dockerfile), production build of the web app (docker/web.Dockerfile)',
      importMs,
      fileInfo: `${path.relative(ROOT, FIXTURE)}, ${boards} boards on ${outline.outline.length} pages; the pan and zoom is on the page ${view.pageName} (${view.boards} boards)`,
      boards,
      boardsOnPage: view.boards,
      runs,
      date,
    });
    await writeFile(REPORT, await spliced(report));

    const m = result.medians;
    console.log('');
    console.log(`cold time to first board: ${m.firstBoardMs.toFixed(1)} ms (limit ${config.limits.firstBoardMs})`);
    console.log(`frame time over the ${config.pathMs / 1000} s pan and zoom: median ${m.frameMedianMs.toFixed(1)} ms, 95th percentile ${m.frameP95Ms.toFixed(1)} ms (limit ${config.limits.frameP95Ms})`);
    console.log(`boards in view at the viewer's minimum zoom: ${m.mostBoards} of ${view.boards} on the page, ${boards} in the file`);
    console.log(`API peak memory: ${m.peakMemoryMiB.toFixed(0)} MiB (limit ${config.limits.peakMemoryMiB})`);
    console.log('wrote docs/bench-ordrar.md');
  } finally {
    await rm(home, { recursive: true, force: true });
    if (process.env.OMNI_BENCH_KEEP !== '1') compose(['down', '--volumes', '--remove-orphans'], { check: false });
  }
}

main().catch((error) => {
  console.error(error.message);
  process.exitCode = 1;
});
