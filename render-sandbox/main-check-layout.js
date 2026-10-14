// The page that renderer gate B (`make check-layout`, check-layout.mjs) uses to
// get the renderer's boxes. It loads the omni-render WebAssembly module that
// `make render-wasm` builds and the stress file, lays the file out with the
// module's `_omni_render_layout_design` export, and puts the box of every node,
// by node ID, in `window.layoutResult`.
import createOmniRender from './build/omni_render.js';

const DESIGN_URL = './build/stress-file.json';

const status = document.querySelector('#status');

try {
  status.textContent = 'Loading the stress file…';
  const response = await fetch(DESIGN_URL);
  if (!response.ok) throw new Error('there is no build/stress-file.json: run `make stress-file` first');
  const bytes = new TextEncoder().encode(await response.text());

  const renderer = await createOmniRender();
  status.textContent = 'Laying out the stress file…';
  const pointer = renderer._omni_render_alloc(bytes.length);
  // HEAPU8 is looked up after the allocation, which may grow the memory.
  renderer.HEAPU8.set(bytes, pointer);
  let boxes;
  try {
    boxes = renderer._omni_render_layout_design(pointer, bytes.length);
  } finally {
    renderer._omni_render_free(pointer, bytes.length);
  }
  if (!boxes) throw new Error(renderer.UTF8ToString(renderer._omni_render_error()));
  const result = JSON.parse(renderer.UTF8ToString(boxes));
  status.textContent = `Laid out ${Object.keys(result).length} nodes.`;
  window.layoutResult = {
    boxes: result,
    devicePixelRatio: window.devicePixelRatio,
    viewport: [window.innerWidth, window.innerHeight],
  };
} catch (error) {
  status.textContent = `The layout failed: ${error.message}`;
  window.layoutResult = { error: error.message };
  throw error;
}
