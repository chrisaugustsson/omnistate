// Loads the omni-render WebAssembly module that `make render-wasm` builds, has
// Skia draw the design of instances.json with the design system of
// instances-system.json on the page's WebGL 2 canvas, and lets the reviewer
// switch between the design system's modes, light and dark. The module expands
// the instances and resolves the tokens; this page only hands it the two files,
// calls `_omni_render_scene_set_mode`, and shows what `_omni_render_scene_errors`
// says is missing (nothing, for the sample).
//
// The module takes text as bytes in its own memory: this page copies it in with
// `_omni_render_alloc` and the exported `HEAPU8`. Click selects a node, and a
// node inside an instance is named `<instanceId>/<componentNodeId>`.
import createOmniRender from './build/omni_render.js';

const DESIGN_URL = './instances.json';
const SYSTEM_URL = './instances-system.json';
const MODES = ['light', 'dark'];

const canvas = document.querySelector('#canvas');
const status = document.querySelector('#status');
const overlay = document.querySelector('#overlay');
const selection = document.querySelector('#selection');
const modes = document.querySelector('#modes');
const errors = document.querySelector('#errors');
const encoder = new TextEncoder();

let renderer;
let gpu = '';
let dirty = true;

function failure() {
  return new Error(renderer.UTF8ToString(renderer._omni_render_error()));
}

// Calls `use(pointer, length)` with `text` copied into the module's memory.
function withText(text, use) {
  const bytes = encoder.encode(text);
  const pointer = renderer._omni_render_alloc(bytes.length);
  // HEAPU8 is looked up after the allocation, which may grow the memory.
  renderer.HEAPU8.set(bytes, pointer);
  try {
    return use(pointer, bytes.length);
  } finally {
    renderer._omni_render_free(pointer, bytes.length);
  }
}

function loadScene(design) {
  if (!withText(design, (pointer, length) => renderer._omni_render_scene_load(pointer, length))) {
    throw failure();
  }
}

function setSystem(system) {
  if (!withText(system, (pointer, length) => renderer._omni_render_scene_set_system(pointer, length))) {
    throw failure();
  }
}

function setMode(mode) {
  if (!withText(mode, (pointer, length) => renderer._omni_render_scene_set_mode(pointer, length))) {
    throw failure();
  }
  document.body.style.background = mode === 'dark' ? '#020617' : '#94a3b8';
  dirty = true;
  showErrors();
  drawOverlay();
}

// Lists what the module drew as an error placeholder.
function showErrors() {
  const list = JSON.parse(renderer.UTF8ToString(renderer._omni_render_scene_errors()));
  errors.classList.toggle('failed', list.length > 0);
  errors.textContent = list.length
    ? `Errors: ${list.map(describe).join('; ')}.`
    : 'Errors: none.';
}

function describe(error) {
  if (error.property) {
    return `${error.node}: ${error.token} does not fit its ${error.property}`;
  }
  return `${error.node} is missing ${error.component ?? error.token}`;
}

function boundsOf(address) {
  return withText(address, (pointer, length) => {
    const result = renderer._omni_render_bounds_of(pointer, length);
    return result ? JSON.parse(renderer.UTF8ToString(result)) : null;
  });
}

function drawOverlay() {
  const outlines = JSON.parse(renderer.UTF8ToString(renderer._omni_render_overlay()));
  overlay.replaceChildren(
    ...outlines.map((outline) => {
      const box = document.createElement('div');
      box.className = `outline ${outline.kind}`;
      if (outline.y < 18) box.classList.add('inside');
      box.style.left = `${outline.x}px`;
      box.style.top = `${outline.y}px`;
      box.style.width = `${outline.width}px`;
      box.style.height = `${outline.height}px`;
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = `${outline.label} (${outline.node})`;
      box.append(label);
      return box;
    }),
  );
}

function takeEvents() {
  const events = JSON.parse(renderer.UTF8ToString(renderer._omni_render_take_events()));
  for (const event of events) {
    if (event.type !== 'select') continue;
    const bounds = event.node && boundsOf(event.node);
    selection.textContent = bounds
      ? `Selected ${event.node}: ${Math.round(bounds.page.width)} x ${Math.round(bounds.page.height)} at ` +
        `${Math.round(bounds.page.x)}, ${Math.round(bounds.page.y)}.`
      : 'Nothing selected.';
  }
  drawOverlay();
}

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
  if (!renderer._omni_render_scene_draw(canvas.width, canvas.height, ratio)) throw failure();
  drawOverlay();
  status.textContent = `Drawn on ${gpu}.`;
}

function loop() {
  if (dirty) {
    dirty = false;
    drawFrame();
  }
  requestAnimationFrame(loop);
}

function position(event) {
  const rect = canvas.getBoundingClientRect();
  return [event.clientX - rect.left, event.clientY - rect.top];
}

function wheel(event) {
  const [x, y] = position(event);
  const unit = [1, 16, canvas.clientHeight][event.deltaMode] ?? 1;
  if (renderer._omni_render_view_wheel(event.deltaX * unit, event.deltaY * unit, x, y, event.ctrlKey, event.metaKey)) {
    event.preventDefault();
    dirty = true;
  }
}

function pointerMove(event) {
  const [x, y] = position(event);
  renderer._omni_render_select_pointer_move(x, y, event.ctrlKey, event.metaKey);
  takeEvents();
}

function click(event) {
  const [x, y] = position(event);
  renderer._omni_render_select_click(x, y, event.ctrlKey, event.metaKey);
  renderer._omni_render_select_pointer_move(x, y, event.ctrlKey, event.metaKey);
  takeEvents();
}

// A radio button for each mode.
function offerModes(initial) {
  modes.replaceChildren('Mode: ');
  for (const mode of MODES) {
    const label = document.createElement('label');
    const radio = document.createElement('input');
    radio.type = 'radio';
    radio.name = 'mode';
    radio.value = mode;
    radio.checked = mode === initial;
    radio.addEventListener('change', () => {
      try {
        setMode(mode);
      } catch (error) {
        status.textContent = `The renderer failed: ${error.message}`;
      }
    });
    label.append(radio, ` ${mode}`);
    modes.append(label);
  }
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

  const [design, system] = await Promise.all(
    [DESIGN_URL, SYSTEM_URL].map(async (url) => {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`cannot load ${url}`);
      return response.text();
    }),
  );
  loadScene(design);
  setSystem(system);
  offerModes(MODES[0]);
  setMode(MODES[0]);

  canvas.addEventListener('wheel', wheel, { passive: false });
  canvas.addEventListener('pointermove', pointerMove);
  canvas.addEventListener('pointerleave', () => {
    // A point that is nowhere, so that hover clears.
    renderer._omni_render_select_pointer_move(NaN, NaN, false, false);
    takeEvents();
  });
  canvas.addEventListener('click', click);
  new ResizeObserver(() => {
    dirty = true;
  }).observe(canvas);

  requestAnimationFrame(loop);
} catch (error) {
  status.textContent = `The renderer failed: ${error.message}`;
  throw error;
}
