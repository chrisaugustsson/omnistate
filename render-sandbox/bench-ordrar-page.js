// What `make bench-ordrar` (bench-ordrar.mjs) injects into the web app's own
// pages before any script of theirs runs. It measures the real design viewer
// without changing it:
//
// - bench-ordrar.mjs serves /render/omni_render.js with `window.__benchHook`
//   called on the renderer module, which is where the viewer's calls into the
//   WebAssembly (`_omni_render_scene_load`, `_scene_draw`, `_view_wheel`) are
//   timed. Every frame the viewer draws ends with `gl.finish()` here, so a frame
//   is complete when the GPU has finished it, as in renderer gate A.
// - JSON.parse and JSON.stringify are timed, for the pages and boards that
//   travel between the API, the page and the renderer.
// - `window.__bench.runPath` plays the scripted pan and zoom with synthetic
//   wheel events on the viewer's canvas, so the path is the same in every run.
//
// Times are `performance.now()` values, which count from the navigation that
// opened the page.
(() => {
  const nativeParse = JSON.parse;
  const nativeStringify = JSON.stringify;
  const bench = (window.__bench = {
    // Work of the page that is not yet part of a frame: the sum of what ran
    // since the last frame ended, which that frame is charged with.
    pending: { json: 0, wheel: 0, parse: 0, layout: 0 },
    // Intervals of the main thread that the phases own, until the first board.
    spans: [],
    firstBoard: null,
    firstBoardInfo: null,
    lastLoadEnd: 0,
    frames: [],
    recording: false,
    ticks: [],
    canvas: null,
  });
  const span = (phase, start, end) => {
    if (bench.firstBoard === null) bench.spans.push({ phase, start, end });
  };

  const timed = (original, self) =>
    function (...args) {
      const start = performance.now();
      try {
        return original.apply(self, args);
      } finally {
        const end = performance.now();
        bench.pending.json += end - start;
        span('parse', start, end);
      }
    };
  JSON.parse = timed(nativeParse, JSON);
  JSON.stringify = timed(nativeStringify, JSON);

  let gl = null;
  const getContext = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (type, ...rest) {
    const context = getContext.call(this, type, ...rest);
    if (type === 'webgl2' && context) {
      gl = context;
      bench.canvas = this;
    }
    return context;
  };

  window.__benchHook = (module) => {
    const read = (pointer) => (pointer ? nativeParse(module.UTF8ToString(pointer)) : null);
    // A page may make more than one renderer, each with a module of its own;
    // the last one is the one that `bench.stats` asks.
    const stats = () => read(module._omni_render_scene_stats());
    bench.stats = stats;
    // Emscripten's export wrappers replace themselves on `module` at their first
    // call, so the wrapper stays in place and takes what is assigned over it as
    // the function it calls.
    const intercept = (name, wrap) => {
      let inner = module[name];
      const wrapped = wrap((...args) => inner(...args));
      Object.defineProperty(module, name, {
        configurable: true,
        enumerable: true,
        get: () => wrapped,
        set: (value) => {
          inner = value;
        },
      });
    };

    intercept('_omni_render_scene_load', (load) => (pointer, length) => {
      const start = performance.now();
      const ok = load(pointer, length);
      const end = performance.now();
      const timings = read(module._omni_render_scene_timings());
      const layout = timings.layoutMs;
      bench.pending.layout += layout;
      bench.pending.parse += end - start - layout;
      span('parse', start, end - layout);
      span('layout', end - layout, end);
      bench.lastLoadEnd = end;
      bench.loads = (bench.loads ?? 0) + 1;
      return ok;
    });

    intercept('_omni_render_view_wheel', (wheel) => (...args) => {
      const start = performance.now();
      const used = wheel(...args);
      bench.pending.wheel += performance.now() - start;
      return used;
    });

    intercept('_omni_render_scene_draw', (draw) => (width, height, ratio) => {
      const start = performance.now();
      const ok = draw(width, height, ratio);
      const drawnAt = performance.now();
      gl.finish();
      const end = performance.now();
      const timings = read(module._omni_render_scene_timings());
      const submit = timings.submitMs;
      const paint = drawnAt - start - submit;
      const gpu = submit + (end - drawnAt);
      span('paint', start, start + paint);
      span('gpu', start + paint, end);
      const { json, wheel: wheelMs, parse, layout } = bench.pending;
      const frame = {
        at: end,
        ms: json + wheelMs + parse + layout + (end - start),
        parse: json + parse,
        layout,
        paint: paint + wheelMs,
        gpu,
      };
      bench.pending = { json: 0, wheel: 0, parse: 0, layout: 0 };
      if (bench.recording) bench.frames.push(frame);
      if (bench.firstBoard === null) {
        // Boards that are not read yet are drawn as placeholders, which have no
        // children, so a frame that painted more nodes than boards drew a board.
        const drawn = stats();
        if (drawn && drawn.nodesPainted > drawn.boardsDrawn) {
          bench.firstBoard = end;
          bench.firstBoardInfo = drawn;
        }
      }
      return ok;
    });
  };

  // The scripted pan and zoom. `cfg` has the canvas bounds of the page's boards
  // (`bounds`), the board the page opens on (`opening`, the first of the page)
  // and the length of the path (`ms`), all in page pixels. The view is steered
  // with the same wheel events a person's trackpad makes: Ctrl and the wheel
  // zoom around the middle of the canvas, the wheel alone pans.
  const WHEEL_ZOOM_RATE = 0.002;
  const MIN_ZOOM = 0.01;
  const MAX_ZOOM = 64;
  const clamp = (zoom) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));

  bench.steer = (cfg, from) => {
    const canvas = bench.canvas;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    const rect = canvas.getBoundingClientRect();
    const { bounds, opening } = cfg;
    const fit = (box) => clamp(Math.min(width / box.width, height / box.height));
    // A page opens on a view that fits all its boards, centered; the path
    // starts from the opening board at board level (`from` is 'board').
    const box = from === 'board' ? opening : bounds;
    const view = {
      x: box.x + box.width / 2,
      y: box.y + box.height / 2,
      zoom: fit(box),
    };
    const send = (deltaX, deltaY, ctrlKey) => {
      canvas.dispatchEvent(
        new WheelEvent('wheel', {
          deltaX,
          deltaY,
          deltaMode: 0,
          ctrlKey,
          clientX: rect.left + width / 2,
          clientY: rect.top + height / 2,
          bubbles: true,
          cancelable: true,
        }),
      );
    };
    const moveTo = (x, y, zoom) => {
      const wanted = clamp(zoom);
      if (Math.abs(Math.log(wanted / view.zoom)) > 1e-6) {
        send(0, -Math.log(wanted / view.zoom) / WHEEL_ZOOM_RATE, true);
        view.zoom = wanted;
      }
      const dx = (x - view.x) * view.zoom;
      const dy = (y - view.y) * view.zoom;
      if (Math.abs(dx) > 1e-3 || Math.abs(dy) > 1e-3) send(dx, dy, false);
      view.x = x;
      view.y = y;
    };
    return { view, moveTo, fit, size: { width, height } };
  };

  bench.runPath = (cfg) =>
    new Promise((resolve) => {
      const { moveTo, fit, size } = bench.steer(cfg, 'board');
      const { bounds, opening } = cfg;
      const board = { x: opening.x + opening.width / 2, y: opening.y + opening.height / 2, zoom: fit(opening) };
      const all = { x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2, zoom: fit(bounds) };
      // Zoomed in four times from the whole page, the screen shows a quarter of
      // the page's width and height: the pan goes along its rows.
      const mid = clamp(all.zoom * 4);
      const halfWidth = size.width / mid / 2;
      const halfHeight = size.height / mid / 2;
      const left = bounds.x + halfWidth;
      const right = bounds.x + bounds.width - halfWidth;
      const top = bounds.y + halfHeight;
      const bottom = bounds.y + bounds.height - halfHeight;
      const middle = (top + bottom) / 2;
      // [time as a share of the path, x, y, zoom]: the opening board, all
      // boards in view, then a pan across the whole page along its top, down
      // its right side and back along its bottom, and in on the opening board.
      const way = [
        [0, board.x, board.y, board.zoom],
        [0.2, all.x, all.y, all.zoom],
        [0.3, left, top, mid],
        [0.5, right, top, mid],
        [0.6, right, bottom, mid],
        [0.75, left, bottom, mid],
        [1, board.x, board.y, board.zoom],
      ];
      // Ease in and out of every leg; the zoom changes by a fixed ratio per
      // time, as the wheel's zoom does.
      const at = (share) => {
        const index = Math.max(1, way.findIndex((point) => point[0] >= share));
        const [t0, x0, y0, z0] = way[index - 1];
        const [t1, x1, y1, z1] = way[index];
        const k = (share - t0) / (t1 - t0);
        const e = k * k * (3 - 2 * k);
        return [x0 + (x1 - x0) * e, y0 + (y1 - y0) * e, Math.exp(Math.log(z0) + (Math.log(z1) - Math.log(z0)) * e)];
      };
      bench.frames = [];
      bench.ticks = [];
      bench.recording = true;
      const started = performance.now();
      const tick = () => {
        const now = performance.now();
        bench.ticks.push(now);
        const share = Math.min(1, (now - started) / cfg.ms);
        const [x, y, zoom] = at(share);
        moveTo(x, y, zoom);
        if (share < 1) {
          requestAnimationFrame(tick);
          return;
        }
        // The last frame is drawn by the viewer's own loop.
        setTimeout(() => {
          bench.recording = false;
          resolve({ frames: bench.frames, ticks: bench.ticks, zoom: bench.stats()?.zoom });
        }, 250);
      };
      requestAnimationFrame(tick);
    });

  // Moves the view to the opening board at board level (the start of the path).
  bench.toOpening = (cfg) => {
    const { moveTo, fit } = bench.steer(cfg, 'all');
    const { opening } = cfg;
    moveTo(opening.x + opening.width / 2, opening.y + opening.height / 2, fit(opening));
  };
})();
