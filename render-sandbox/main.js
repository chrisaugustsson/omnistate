// Loads the omni-render WebAssembly module that `make render-wasm` builds and
// has Skia draw the sandbox scene on the page's WebGL 2 canvas.
import createOmniRender from './build/omni_render.js';

const canvas = document.querySelector('#canvas');
const status = document.querySelector('#status');

try {
  const renderer = await createOmniRender();

  const pixelRatio = window.devicePixelRatio || 1;
  canvas.width = Math.round(canvas.clientWidth * pixelRatio);
  canvas.height = Math.round(canvas.clientHeight * pixelRatio);

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
  // Skia starts. The status line shows the name too.
  const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
  const context = renderer.GL.registerContext(gl, { majorVersion: 2, minorVersion: 0 });
  renderer.GL.makeContextCurrent(context);

  if (!renderer._omni_render_draw_sandbox(canvas.width, canvas.height, pixelRatio)) {
    throw new Error('Skia could not draw on the WebGL context');
  }
  const gpu = gl.getParameter(debugInfo ? debugInfo.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
  status.textContent = `Drawn by Skia through WebGL 2 on ${gpu}.`;
} catch (error) {
  status.textContent = `The renderer failed: ${error.message}`;
  throw error;
}
