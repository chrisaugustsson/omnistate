// The page that renderer gate A measures (`make bench-render` drives it through
// bench-render.mjs). It loads the omni-render WebAssembly module and the stress
// file, takes one measurement chosen by the `measure` query parameter, and puts
// the result in `window.benchResult`, so that every measurement comes from a
// cold page load:
//
// - `board`: the time from the data of the stress file's 2,000-node board being
//   loaded to the first complete frame of that board.
// - `file`: the time from the data of the whole stress file being loaded to the
//   first frame showing all 100 boards.
// - `frames`: the 95th-percentile frame time of a scripted zoom and pan path
//   over the stress file.
//
// A frame is complete when the GPU has finished it: every timed frame ends with
// `gl.finish()`. The first frame of a measurement includes the work that Skia
// does once, such as compiling shaders and making the snapshots of the boards.
import createOmniRender from './build/omni_render.js';
import { frameP95 } from './bench-render-lib.mjs';

const DESIGN_URL = './build/stress-file.json';
// The board that the stress file makes text heavy, and so the largest.
const BOARD_NODES = 2000;

const canvas = document.querySelector('#canvas');
const status = document.querySelector('#status');
const encoder = new TextEncoder();

let renderer;
let gl;
// How the last first frame split between loading the scene and drawing it.
let parts = {};

function failure() {
  return new Error(renderer.UTF8ToString(renderer._omni_render_error()));
}

function loadScene(json) {
  const bytes = encoder.encode(json);
  const pointer = renderer._omni_render_alloc(bytes.length);
  // HEAPU8 is looked up after the allocation, which may grow the memory.
  renderer.HEAPU8.set(bytes, pointer);
  try {
    if (!renderer._omni_render_scene_load(pointer, bytes.length)) throw failure();
  } finally {
    renderer._omni_render_free(pointer, bytes.length);
  }
}

function stats() {
  return JSON.parse(renderer.UTF8ToString(renderer._omni_render_scene_stats()));
}

// Draws a frame and waits for the GPU to finish it. Reading a pixel back makes
// sure that the work is done, whatever gl.finish does in the browser.
const pixel = new Uint8Array(4);
function drawFrame() {
  const ratio = window.devicePixelRatio;
  if (!renderer._omni_render_scene_draw(canvas.width, canvas.height, ratio)) throw failure();
  gl.finish();
  gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
}

// The number of nodes of `node`, itself and all its descendants.
function count(node) {
  return 1 + (node.children ?? []).reduce((sum, child) => sum + count(child), 0);
}

// The board of the page with exactly BOARD_NODES nodes, as a design of its own.
function biggestBoard(page) {
  const board = page.children.find((child) => count(child) === BOARD_NODES);
  if (!board) throw new Error(`the stress file has no board of ${BOARD_NODES} nodes`);
  return board;
}

// From the data being loaded to the first complete frame, which must show
// `boards` boards.
function firstFrame(json, boards) {
  const started = performance.now();
  loadScene(json);
  const loaded = performance.now();
  drawFrame();
  const elapsed = performance.now() - started;
  parts = { loadMs: loaded - started, drawMs: performance.now() - loaded };
  const drawn = stats().boardsDrawn;
  if (drawn !== boards) {
    throw new Error(`the first frame showed ${drawn} boards, not ${boards}`);
  }
  return elapsed;
}

// The scripted path: zoom in on the middle of the screen to 100%, pan across the
// boards, and zoom out again. Every step is a wheel event and a frame; the
// frame time includes handling the event.
function zoomAndPan() {
  const x = canvas.clientWidth / 2;
  const y = canvas.clientHeight / 2;
  const frameTimes = [];
  const step = (deltaX, deltaY, zoom) => {
    const started = performance.now();
    renderer._omni_render_view_wheel(deltaX, deltaY, x, y, zoom, false);
    drawFrame();
    frameTimes.push(performance.now() - started);
  };
  // Zoom is exponential in the wheel delta, so a fixed delta is a fixed ratio.
  let zoomSteps = 0;
  while (stats().zoom < 1 && zoomSteps < 400) {
    step(0, -12, true);
    zoomSteps += 1;
  }
  for (let i = 0; i < 240; i += 1) step(30, i % 2 === 0 ? 10 : -6, false);
  for (let i = 0; i < zoomSteps; i += 1) step(0, 12, true);
  return { frameTimes, zoomSteps };
}

async function measure(kind) {
  status.textContent = 'Loading the stress file…';
  const response = await fetch(DESIGN_URL);
  if (!response.ok) throw new Error('there is no build/stress-file.json: run `make stress-file` first');
  const json = await response.text();
  // The canvas is as large as the viewport, in device pixels, from the start.
  canvas.width = Math.round(canvas.clientWidth * window.devicePixelRatio);
  canvas.height = Math.round(canvas.clientHeight * window.devicePixelRatio);
  status.textContent = `Measuring ${kind}…`;
  // Let the browser show the message before the module is busy.
  await new Promise((resolve) => requestAnimationFrame(resolve));

  if (kind === 'board') {
    const board = JSON.stringify(biggestBoard(JSON.parse(json)));
    return { boardMs: firstFrame(board, 1) };
  }
  if (kind === 'file') {
    return { fileMs: firstFrame(json, 100) };
  }
  if (kind === 'frames') {
    firstFrame(json, 100);
    const { frameTimes, zoomSteps } = zoomAndPan();
    return { frameP95Ms: frameP95(frameTimes), frames: frameTimes.length, zoomSteps };
  }
  throw new Error(`unknown measurement "${kind}"`);
}

try {
  const kind = new URLSearchParams(location.search).get('measure');
  renderer = await createOmniRender();
  // omni-render draws on the default framebuffer of the current WebGL context:
  // WebGL 2 with an alpha channel, an 8-bit stencil buffer and no multisampling.
  gl = canvas.getContext('webgl2', { alpha: true, depth: false, stencil: true, antialias: false });
  if (!gl) throw new Error('this browser has no WebGL 2');
  const context = renderer.GL.registerContext(gl, { majorVersion: 2, minorVersion: 0 });
  renderer.GL.makeContextCurrent(context);

  const result = await measure(kind);
  status.textContent = `${kind}: ${JSON.stringify(result)}`;
  window.benchResult = {
    ...result,
    parts,
    devicePixelRatio: window.devicePixelRatio,
    viewport: [window.innerWidth, window.innerHeight],
  };
} catch (error) {
  status.textContent = `The measurement failed: ${error.message}`;
  window.benchResult = { error: error.message };
  throw error;
}
