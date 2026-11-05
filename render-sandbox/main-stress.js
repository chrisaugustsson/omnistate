// Loads the omni-render WebAssembly module that `make render-wasm` builds, has
// Skia draw the stress file (`make stress-file` writes it) on the page's WebGL 2
// canvas, and forwards the browser's wheel, pointer, touch and key events to the
// module's view controls, which decide what zooming and panning do.
//
// Zoom with a pinch or Ctrl/Cmd + wheel, pan with the wheel, or by dragging while
// the space key is held. The module takes the design as bytes in its own memory:
// this page copies it in with `_omni_render_alloc` and the exported `HEAPU8`.
//
// Click, double-click, Esc and Ctrl/Cmd + click select nodes, and the pointer
// hovers them. The module keeps the selection and tells this page what changed
// (`_omni_render_take_events`); this page draws the outlines it asks for
// (`_omni_render_overlay`) in the #overlay element above the canvas, once for
// every frame, so that they follow the nodes while zooming and panning. The
// events also leave the page as `omni-hover` and `omni-select` events of the
// canvas, for a host that embeds the renderer.
import createOmniRender from './build/omni_render.js';

const DESIGN_URL = './build/stress-file.json';

const canvas = document.querySelector('#canvas');
const status = document.querySelector('#status');
const overlay = document.querySelector('#overlay');
const selection = document.querySelector('#selection');
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

// Calls an export that takes text, and returns the NUL-ended JSON text it gave
// back parsed, or null if it gave none.
function callWithText(name, text) {
  const bytes = encoder.encode(text);
  const pointer = renderer._omni_render_alloc(bytes.length);
  renderer.HEAPU8.set(bytes, pointer);
  try {
    const result = renderer[name](pointer, bytes.length);
    return result ? JSON.parse(renderer.UTF8ToString(result)) : null;
  } finally {
    renderer._omni_render_free(pointer, bytes.length);
  }
}

// The bounds of the node at an address, in page and in canvas pixels, or null.
function boundsOf(address) {
  return callWithText('_omni_render_bounds_of', address);
}

// Draws the outlines of the hovered and the selected node above the canvas.
function drawOverlay() {
  const outlines = JSON.parse(renderer.UTF8ToString(renderer._omni_render_overlay()));
  overlay.replaceChildren(
    ...outlines.map((outline) => {
      const box = document.createElement('div');
      box.className = `outline ${outline.kind}`;
      // A label above the top edge would be cut off at the top of the screen.
      if (outline.y < 18) box.classList.add('inside');
      box.style.left = `${outline.x}px`;
      box.style.top = `${outline.y}px`;
      box.style.width = `${outline.width}px`;
      box.style.height = `${outline.height}px`;
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = outline.label;
      box.append(label);
      return box;
    }),
  );
}

// Picks up what the module says changed about hover and selection.
function takeEvents() {
  const events = JSON.parse(renderer.UTF8ToString(renderer._omni_render_take_events()));
  for (const event of events) {
    canvas.dispatchEvent(new CustomEvent(`omni-${event.type}`, { detail: event.node }));
    if (event.type !== 'select') continue;
    const bounds = event.node && boundsOf(event.node);
    selection.textContent = bounds
      ? `Selected ${event.node}: ${Math.round(bounds.page.width)} x ${Math.round(bounds.page.height)} at ` +
        `${Math.round(bounds.page.x)}, ${Math.round(bounds.page.y)}.`
      : 'Nothing selected.';
  }
  drawOverlay();
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

  drawOverlay();

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

// Where the pointer last was and whether Ctrl or Cmd is held, so that hover
// can be worked out again when the selection or the keys change.
let pointer = null;
let deepKeys = { ctrl: false, meta: false };
// Whether the click that follows the pointer going up belongs to a pan or a
// pinch, and is not a click on a node.
let ignoreClick = false;

function hover() {
  if (pointer) {
    renderer._omni_render_select_pointer_move(pointer[0], pointer[1], deepKeys.ctrl, deepKeys.meta);
  }
}

function setKeys(event) {
  const keys = { ctrl: event.ctrlKey, meta: event.metaKey };
  if (keys.ctrl === deepKeys.ctrl && keys.meta === deepKeys.meta) return;
  deepKeys = keys;
  hover();
  takeEvents();
}

function pointerDown(event) {
  const [x, y] = position(event);
  canvas.setPointerCapture(event.pointerId);
  touches.set(event.pointerId, { x, y });
  ignoreClick = touches.size >= 2;
  if (touches.size === 2) {
    pinch = between();
  } else if (renderer._omni_render_view_pointer_down(x, y, event.button === 1)) {
    // The middle button would otherwise start the browser's autoscroll.
    if (event.button === 1) event.preventDefault();
    ignoreClick = true;
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
  } else if (touches.size <= 1) {
    pointer = [x, y];
    deepKeys = { ctrl: event.ctrlKey, meta: event.metaKey };
    hover();
    takeEvents();
  }
}

// A click selects the node the pointer is over, as hover showed, unless it
// ended a pan or a pinch.
function click(event) {
  const [x, y] = position(event);
  if (ignoreClick) {
    ignoreClick = false;
    return;
  }
  renderer._omni_render_select_click(x, y, event.ctrlKey, event.metaKey);
  // The context changed, so hover is what a click would select now.
  pointer = [x, y];
  hover();
  takeEvents();
}

function doubleClick(event) {
  const [x, y] = position(event);
  renderer._omni_render_select_double_click(x, y);
  pointer = [x, y];
  hover();
  takeEvents();
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
  canvas.addEventListener('pointerleave', () => {
    pointer = null;
    // A point that is nowhere, so that hover clears.
    renderer._omni_render_select_pointer_move(NaN, NaN, false, false);
    takeEvents();
  });
  canvas.addEventListener('click', click);
  canvas.addEventListener('dblclick', doubleClick);

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
    setKeys(event);
    if (event.key === 'Escape') {
      renderer._omni_render_select_escape();
      hover();
      takeEvents();
    }
    if (event.code !== 'Space') return;
    // Space would scroll the page.
    event.preventDefault();
    setSpace(true);
  });
  window.addEventListener('keyup', (event) => {
    setKeys(event);
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
