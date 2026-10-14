// Loads the omni-render WebAssembly module that `make render-wasm` builds, has
// Skia draw the stress file (`make stress-file` writes it) on the page's WebGL 2
// canvas, and forwards the browser's wheel, pointer, touch and key events to the
// module's view controls, which decide what zooming and panning do.
//
// Zoom with a pinch or Ctrl/Cmd + wheel, pan with the wheel, or by dragging while
// the space key is held. The module takes the design as bytes in its own memory:
// this page copies it in with `_omni_render_alloc` and the exported `HEAPU8`.
import createOmniRender from './build/omni_render.js';

const DESIGN_URL = './build/stress-file.json';

const canvas = document.querySelector('#canvas');
const status = document.querySelector('#status');
const encoder = new TextEncoder();

let renderer;
// The name of the GPU underneath, for the status line.
let gpu = '';
// Whether the view changed since the last frame.
let dirty = true;
// When the last frames were drawn, to report how many a second the page draws.
const frameTimes = [];

function failure() {
  return new Error(renderer.UTF8ToString(renderer._omni_render_error()));
}

// Loads the design into the module's scene.
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

// Makes the drawing buffer as large as the canvas is on the screen, in device pixels.
function sizeCanvas() {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
    dirty = true;
  }
  return ratio;
}

function drawFrame() {
  const ratio = sizeCanvas();
  const started = performance.now();
  if (!renderer._omni_render_scene_draw(canvas.width, canvas.height, ratio)) throw failure();
  const finished = performance.now();
  frameTimes.push(finished);
  while (frameTimes.length > 1 && finished - frameTimes[0] > 1000) frameTimes.shift();

  const stats = JSON.parse(renderer.UTF8ToString(renderer._omni_render_scene_stats()));
  status.textContent =
    `Zoom ${(stats.zoom * 100).toFixed(1)}%, ${stats.detail} detail: ` +
    `${stats.boardsDrawn} of 100 boards in view, ${stats.nodesPainted} nodes painted this frame. ` +
    `Frame ${(finished - started).toFixed(1)} ms on the CPU, ${frameTimes.length} drawn in the last second. (${gpu})`;
}

function requestFrame() {
  dirty = true;
}

function loop() {
  if (dirty) {
    dirty = false;
    drawFrame();
  }
  requestAnimationFrame(loop);
}

// The pointer position in CSS pixels from the canvas's top-left corner.
function position(event) {
  const rect = canvas.getBoundingClientRect();
  return [event.clientX - rect.left, event.clientY - rect.top];
}

// Wheel deltas in CSS pixels, whatever unit the browser reports them in.
function pixels(event) {
  const unit = [1, 16, canvas.clientHeight][event.deltaMode] ?? 1;
  return [event.deltaX * unit, event.deltaY * unit];
}

function wheel(event) {
  const [x, y] = position(event);
  const [deltaX, deltaY] = pixels(event);
  // Ctrl + wheel is also how browsers report a trackpad pinch.
  if (renderer._omni_render_view_wheel(deltaX, deltaY, x, y, event.ctrlKey, event.metaKey)) {
    // The browser must not scroll the page or zoom it.
    event.preventDefault();
    requestFrame();
  }
}

// The fingers on the canvas, by pointer ID, and where they were at the last
// event, to turn two of them into a pinch.
const touches = new Map();
let pinch = null;

function between() {
  const [a, b] = [...touches.values()];
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2, distance: Math.hypot(a.x - b.x, a.y - b.y) };
}

function pointerDown(event) {
  const [x, y] = position(event);
  canvas.setPointerCapture(event.pointerId);
  touches.set(event.pointerId, { x, y });
  if (touches.size === 2) {
    pinch = between();
  } else if (renderer._omni_render_view_pointer_down(x, y)) {
    canvas.classList.add('grabbing');
  }
}

function pointerMove(event) {
  const [x, y] = position(event);
  if (touches.has(event.pointerId)) touches.set(event.pointerId, { x, y });
  if (touches.size === 2 && pinch) {
    const now = between();
    if (pinch.distance > 0 && now.distance > 0) {
      renderer._omni_render_view_pinch(now.distance / pinch.distance, now.x, now.y);
    }
    // The fingers moving together pan, as a scroll against their motion does.
    renderer._omni_render_view_wheel(pinch.x - now.x, pinch.y - now.y, now.x, now.y, false, false);
    pinch = now;
    requestFrame();
  } else if (touches.size <= 1 && renderer._omni_render_view_pointer_move(x, y)) {
    requestFrame();
  }
}

function pointerUp(event) {
  touches.delete(event.pointerId);
  pinch = null;
  renderer._omni_render_view_pointer_up();
  canvas.classList.remove('grabbing');
}

function setSpace(held) {
  renderer._omni_render_view_space(held);
  canvas.classList.toggle('panning', held);
  if (!held) canvas.classList.remove('grabbing');
}

try {
  renderer = await createOmniRender();

  // omni-render draws on the default framebuffer of the current WebGL
  // context: WebGL 2 with an alpha channel, an 8-bit stencil buffer and no
  // multisampling (Skia anti-aliases by itself).
  const gl = canvas.getContext('webgl2', {
    alpha: true,
    depth: false,
    stencil: true,
    antialias: false,
  });
  if (!gl) {
    throw new Error('this browser has no WebGL 2');
  }
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  const context = renderer.GL.registerContext(gl, { majorVersion: 2, minorVersion: 0 });
  renderer.GL.makeContextCurrent(context);
  gpu = gl.getParameter(debugInfo ? debugInfo.UNMASKED_RENDERER_WEBGL : gl.RENDERER);

  status.textContent = 'Loading the stress file…';
  const response = await fetch(DESIGN_URL);
  if (!response.ok) {
    throw new Error('there is no build/stress-file.json: run `make stress-file` first');
  }
  const json = await response.text();
  status.textContent = 'Laying out the stress file…';
  // Let the browser show the message before the module is busy.
  await new Promise((resolve) => requestAnimationFrame(resolve));
  loadScene(json);

  status.textContent = 'Drawing the first frame, which makes the snapshots…';
  await new Promise((resolve) => requestAnimationFrame(resolve));
  drawFrame();

  // The wheel listener is not passive, so that preventDefault can keep the
  // browser from zooming or scrolling the page.
  canvas.addEventListener('wheel', wheel, { passive: false });
  canvas.addEventListener('pointerdown', pointerDown);
  canvas.addEventListener('pointermove', pointerMove);
  canvas.addEventListener('pointerup', pointerUp);
  canvas.addEventListener('pointercancel', pointerUp);

  // Safari reports a trackpad pinch as gesture events instead of Ctrl + wheel.
  let gestureScale = 1;
  canvas.addEventListener('gesturestart', (event) => {
    event.preventDefault();
    gestureScale = 1;
  });
  canvas.addEventListener('gesturechange', (event) => {
    event.preventDefault();
    const [x, y] = position(event);
    renderer._omni_render_view_pinch(event.scale / gestureScale, x, y);
    gestureScale = event.scale;
    requestFrame();
  });

  window.addEventListener('keydown', (event) => {
    if (event.code !== 'Space') return;
    // Space would scroll the page.
    event.preventDefault();
    setSpace(true);
  });
  window.addEventListener('keyup', (event) => {
    if (event.code === 'Space') setSpace(false);
  });
  window.addEventListener('blur', () => setSpace(false));
  new ResizeObserver(requestFrame).observe(canvas);
  window.addEventListener('resize', requestFrame);

  requestAnimationFrame(loop);
} catch (error) {
  status.textContent = `The renderer failed: ${error.message}`;
  throw error;
}
