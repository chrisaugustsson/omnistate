// Loads the omni-render WebAssembly module that `make render-wasm` builds, has
// Skia draw the design of nodes.json on the page's WebGL 2 canvas, and measures
// how far the layout of this build is from the native build's.
//
// The module takes designs, images and fonts as bytes in its own memory: this
// page copies them in with `_omni_render_alloc` and the exported `HEAPU8`.
import createOmniRender from './build/omni_render.js';

const DESIGN_URL = './nodes.json';
// The boxes of the native build (CoreText), which `make nodes-boxes` writes.
const NATIVE_BOXES_URL = './build/nodes-boxes.json';

// The images that the attachments of nodes.json name, which the host supplies.
// The attachment of the last image node is left out on purpose.
const PHOTO = 'attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd';
const PORTRAIT = 'attachment:0192f1c4-5b6a-7d8e-9f01-23456789abce';

const canvas = document.querySelector('#canvas');
const status = document.querySelector('#status');
const layoutStatus = document.querySelector('#layout');
const fontInput = document.querySelector('#font');
const encoder = new TextEncoder();

let renderer;
let design;
let designJson;

// Calls `use(pointer, length)` with `bytes` copied into the module's memory.
function withBytes(bytes, use) {
  const pointer = renderer._omni_render_alloc(bytes.length);
  // HEAPU8 is looked up after the allocation, which may grow the memory.
  renderer.HEAPU8.set(bytes, pointer);
  try {
    return use(pointer, bytes.length);
  } finally {
    renderer._omni_render_free(pointer, bytes.length);
  }
}

function withText(text, use) {
  return withBytes(encoder.encode(text), use);
}

function failure() {
  return new Error(renderer.UTF8ToString(renderer._omni_render_error()));
}

function addImage(attachment, bytes) {
  const added = withText(attachment, (name, nameLength) =>
    withBytes(bytes, (data, dataLength) =>
      renderer._omni_render_add_image(name, nameLength, data, dataLength)));
  if (!added) throw failure();
}

function addFont(family, bytes) {
  const added = withText(family, (name, nameLength) =>
    withBytes(bytes, (data, dataLength) =>
      renderer._omni_render_add_font(name, nameLength, data, dataLength)));
  if (!added) throw failure();
}

// The boxes of every node, by ID, as the module lays the design out.
function layoutDesign() {
  const boxes = withText(designJson, (pointer, length) =>
    renderer._omni_render_layout_design(pointer, length));
  if (!boxes) throw failure();
  return JSON.parse(renderer.UTF8ToString(boxes));
}

function draw(width, height, pixelRatio) {
  const drawn = withText(designJson, (pointer, length) =>
    renderer._omni_render_draw_design(pointer, length, width, height, pixelRatio));
  if (!drawn) throw failure();
}

function pictureBytes(width, height, paint) {
  const picture = document.createElement('canvas');
  picture.width = width;
  picture.height = height;
  paint(picture.getContext('2d'));
  return new Promise((resolve, reject) => {
    picture.toBlob(async (blob) => {
      if (!blob) return reject(new Error('the browser could not encode a PNG'));
      resolve(new Uint8Array(await blob.arrayBuffer()));
    }, 'image/png');
  });
}

// A 2:1 picture of four colored columns with a white circle on them, which
// shows how `cover` and `contain` crop and center it.
function paintPhoto(context) {
  ['#dc2828', '#28a03c', '#2850dc', '#f0c81e'].forEach((color, column) => {
    context.fillStyle = color;
    context.fillRect(column * 50, 0, 50, 100);
  });
  context.fillStyle = '#ffffff';
  context.beginPath();
  context.arc(100, 50, 30, 0, 2 * Math.PI);
  context.fill();
}

// A 1:2 picture, blue above yellow.
function paintPortrait(context) {
  context.fillStyle = '#2850dc';
  context.fillRect(0, 0, 100, 100);
  context.fillStyle = '#f0c81e';
  context.fillRect(0, 100, 100, 100);
}

// The width and height that the boards of the design take on the page.
function extent(boxes) {
  let width = 0;
  let height = 0;
  for (const board of design.children) {
    const box = boxes[board.id];
    width = Math.max(width, Math.ceil(box.x + box.width));
    height = Math.max(height, Math.ceil(box.y + box.height));
  }
  return { width, height };
}

// Draws the design on a WebGL canvas that is as large as its boards.
function redraw(gl) {
  const { width, height } = extent(layoutDesign());
  // A canvas cannot be larger than the GPU's biggest texture.
  const limit = gl.getParameter(gl.MAX_TEXTURE_SIZE);
  const pixelRatio = Math.min(window.devicePixelRatio || 1, limit / width, limit / height);
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.width = Math.round(width * pixelRatio);
  canvas.height = Math.round(height * pixelRatio);
  draw(canvas.width, canvas.height, pixelRatio);
}

// The names of the nodes by ID, for the report of the layout comparison.
function namesOf(node, names = {}) {
  names[node.id] = node.name;
  for (const child of node.children ?? []) namesOf(child, names);
  return names;
}

// How far apart the WebAssembly build (FreeType) and the native build
// (CoreText) lay the design out: the largest difference of any x, y, width or
// height of any node, in CSS pixels.
async function compareLayouts() {
  const response = await fetch(NATIVE_BOXES_URL);
  if (!response.ok) {
    layoutStatus.textContent = 'Run `make nodes-boxes` to compare this build’s layout with the native build’s.';
    return;
  }
  const native = await response.json();
  const wasm = layoutDesign();
  const names = namesOf(design);
  let largest = { difference: 0, id: design.id, property: 'x' };
  let compared = 0;
  for (const [id, box] of Object.entries(native)) {
    for (const property of ['x', 'y', 'width', 'height']) {
      const difference = Math.abs(box[property] - wasm[id][property]);
      if (difference > largest.difference) largest = { difference, id, property };
    }
    compared += 1;
  }
  layoutStatus.textContent =
    `Layout of ${compared} nodes, WebAssembly (FreeType) against native (CoreText): ` +
    `the largest difference is ${largest.difference === 0 ? '0' : largest.difference.toPrecision(3)} px` +
    (largest.difference > 0 ? ` (${largest.property} of “${names[largest.id]}”).` : '.');
}

try {
  renderer = await createOmniRender();
  designJson = await (await fetch(DESIGN_URL)).text();
  design = JSON.parse(designJson);

  addImage(PHOTO, await pictureBytes(200, 100, paintPhoto));
  addImage(PORTRAIT, await pictureBytes(100, 200, paintPortrait));

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
  // Skia asks for the GPU name through this extension, so enable it before
  // Skia starts.
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  const context = renderer.GL.registerContext(gl, { majorVersion: 2, minorVersion: 0 });
  renderer.GL.makeContextCurrent(context);

  redraw(gl);
  const gpu = gl.getParameter(debugInfo ? debugInfo.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
  status.textContent = `Drawn by Skia through WebGL 2 on ${gpu}.`;
  await compareLayouts();

  fontInput.addEventListener('change', async () => {
    const [file] = fontInput.files;
    if (!file) return;
    try {
      addFont('Missing Sans', new Uint8Array(await file.arrayBuffer()));
      redraw(gl);
      status.textContent = `Drawn again with ${file.name} as “Missing Sans”.`;
    } catch (error) {
      status.textContent = `The font was not supplied: ${error.message}`;
    }
  });
} catch (error) {
  status.textContent = `The renderer failed: ${error.message}`;
  throw error;
}
