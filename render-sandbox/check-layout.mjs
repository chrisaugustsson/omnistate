// `make check-layout`: renderer gate B (US-009), the layout parity check. It
// exports the stress file as HTML with the export of `make chrome-boxes`
// (layout-export.mjs, through check-layout-lib.mjs), renders it in headed
// Chrome, through Playwright, at the viewport and device pixel ratio of gate A,
// and reads every node's box with `getBoundingClientRect`. check-layout.html
// lays the same file out with the WebAssembly build of the renderer in a page
// of the same Chrome, and every node's box is compared with Chrome's. The
// result goes to the Gate B section of docs/renderer-gate.md, after Gate A. The
// exit status is 0 only if every node is within the tolerance.
//
// `make check-layout` builds the inputs (build/omni_render.wasm and
// build/stress-file.json) and installs Playwright with npm first. It drives the
// Chrome that is installed on the machine (`channel: 'chrome'`). `CHROME` names
// another Chrome binary.
import { createReadStream, existsSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';
import { compareBoxes, config, prepareExport, renderReport, spliceReport } from './check-layout-lib.mjs';
import { boxesOf } from './layout-export.mjs';

const SANDBOX = path.dirname(fileURLToPath(import.meta.url));
const REPORT = path.join(SANDBOX, '..', 'docs', 'renderer-gate.md');
const DESIGN = path.join(SANDBOX, 'build', 'stress-file.json');
const TITLE = '# Renderer gate\n\nThe pass/fail tests of the renderer (Q35). Each section is written by the script of its gate.\n';
// How long a page may take to load, lay the file out and be measured.
const PAGE_TIMEOUT_MS = 170_000;

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
};

// Serves render-sandbox/ on a free local port, and the export of the design at
// /export.html.
function serve(exportHtml) {
  const server = http.createServer((request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (pathname === '/export.html') {
      response.writeHead(200, { 'content-type': TYPES['.html'], 'cache-control': 'no-store' });
      response.end(exportHtml);
      return;
    }
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

// The rectangle of every node of the export, `[x, y, width, height]` by node
// ID, once the bundled fonts have loaded, so that no text is measured in a
// fallback font.
function measureChrome(page) {
  return page.evaluate(async () => {
    await Promise.all([document.fonts.load('16px Inter'), document.fonts.load('italic 16px Inter')]);
    await document.fonts.ready;
    const unloaded = [...document.fonts].filter((face) => face.status !== 'loaded');
    if (unloaded.length > 0) {
      throw new Error(`the bundled fonts did not load: ${unloaded.map((face) => `${face.family} ${face.status}`)}`);
    }
    const rects = {};
    for (const element of document.querySelectorAll('[data-node]')) {
      const rect = element.getBoundingClientRect();
      rects[element.dataset.node] = [rect.x + scrollX, rect.y + scrollY, rect.width, rect.height];
    }
    return { rects, devicePixelRatio, viewport: [innerWidth, innerHeight] };
  });
}

function machineName() {
  return `${os.cpus()[0].model}, ${Math.round(os.totalmem() / 2 ** 30)} GB memory, ${os.type()} ${os.release()} (${os.arch()})`;
}

function checkPage({ devicePixelRatio, viewport }, what) {
  const { width, height } = config.viewport;
  if (devicePixelRatio !== config.deviceScaleFactor || viewport.join('x') !== `${width}x${height}`) {
    throw new Error(
      `${what} ran at ${viewport.join('x')} and a pixel ratio of ${devicePixelRatio}, not ${width}x${height} at ${config.deviceScaleFactor}`,
    );
  }
}

if (!existsSync(DESIGN)) {
  throw new Error('there is no build/stress-file.json: run `make check-layout`, which writes it');
}
const design = JSON.parse(await readFile(DESIGN, 'utf8'));
const { html } = prepareExport(design);
const { server, port } = await serve(html);
const base = `http://127.0.0.1:${port}`;
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

  console.log('rendering the export in Chrome…');
  const chromePage = await context.newPage();
  chromePage.setDefaultTimeout(PAGE_TIMEOUT_MS);
  chromePage.on('pageerror', (error) => console.error(`  page error: ${error.message}`));
  await chromePage.goto(`${base}/export.html`, { waitUntil: 'load', timeout: PAGE_TIMEOUT_MS });
  const measured = await measureChrome(chromePage);
  checkPage(measured, 'the export');
  const chrome = boxesOf(design, measured.rects);
  await chromePage.close();

  console.log('laying the stress file out with the WebAssembly build…');
  const rendererPage = await context.newPage();
  rendererPage.on('pageerror', (error) => console.error(`  page error: ${error.message}`));
  await rendererPage.goto(`${base}/check-layout.html`);
  await rendererPage.waitForFunction(() => window.layoutResult, null, { timeout: PAGE_TIMEOUT_MS });
  const laidOut = await rendererPage.evaluate(() => window.layoutResult);
  if (laidOut.error) throw new Error(`the layout failed in the page: ${laidOut.error}`);
  checkPage(laidOut, 'the renderer page');

  const comparison = compareBoxes(chrome, laidOut.boxes, config.tolerancePx);
  const date = new Date().toISOString().slice(0, 10);
  const report = renderReport({
    machine: machineName(),
    chrome: `Chrome ${browser.version()}`,
    date,
    comparison,
  });
  const old = existsSync(REPORT) ? await readFile(REPORT, 'utf8') : TITLE;
  await writeFile(REPORT, spliceReport(old, report));

  const { largest, mismatches, compared } = comparison;
  console.log(`compared ${compared} nodes: ${mismatches.length} beyond ${config.tolerancePx} px, largest difference ${largest.difference} px (${largest.property} of ${largest.id})`);
  for (const mismatch of mismatches.slice(0, 20)) {
    console.log(`  ${mismatch.id}: ${mismatch.property} differs by ${mismatch.difference}`);
  }
  console.log(`wrote docs/renderer-gate.md: the gate ${comparison.pass ? 'PASSED' : 'FAILED'}`);
  process.exitCode = comparison.pass ? 0 : 1;
} finally {
  await browser.close();
  server.close();
}
