// What `make bench-ordrar` (bench-ordrar.mjs) injects into the web app's own
// pages before any script of theirs runs. It measures the real design viewer
// without changing it:
//
// - bench-ordrar.mjs serves /render/omni_render.js with `window.__benchHook`
//   called on the renderer module, which is where every call of the viewer into
//   the WebAssembly (the `_omni_render_*` exports) is timed. Every frame the
//   viewer draws ends with `gl.finish()` here, so a frame is complete when the
//   GPU has finished it, as in renderer gate A. A frame's time is the time since
//   the previous complete frame, so it holds whatever kept the next frame from
//   being drawn, timed or not.
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
    // since the last frame ended, which that frame is charged with. Time that
    // no phase holds is the frame's `other`.
    pending: { json: 0, wheel: 0, parse: 0, layout: 0 },
    // When the last frame ended.
    since: 0,
    // Time in each renderer export while the path plays: { name: { calls, ms } }.
    calls: {},
    // The main thread's long tasks, which hold what the page cannot time.
    longTasks: [],
    // Intervals of the main thread that the phases own, until the first board.
    spans: [],
    firstBoard: null,
    firstBoardInfo: null,
    lastLoadEnd: 0,
    frames: [],
    recording: false,
    // Called at the next complete frame, which ends the path.
    onFrame: null,
    ticks: [],
    canvas: null,
  });
  const span = (phase, start, end) => {
    if (bench.firstBoard === null) bench.spans.push({ phase, start, end });
  };
  new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      bench.longTasks.push({ phase: 'other', start: entry.startTime, end: entry.startTime + entry.duration });
    }
  }).observe({ type: 'longtask', buffered: true });

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

    // A page may make more than one renderer, each with a module of its own;
    // the last one is the one that `bench.stats` asks. What the bench asks
    // itself is not timed.
    const stats = module._omni_render_scene_stats;
    const timings = module._omni_render_scene_timings;
    bench.stats = () => read(stats());
    const unmeasured = new Set(['_omni_render_scene_stats', '_omni_render_scene_timings']);

    // Reading the page or the design system, then laying the scene out.
    const load = (start, end) => {
      const layout = read(timings()).layoutMs;
      bench.pending.layout += layout;
      bench.pending.parse += end - start - layout;
      span('parse', start, end - layout);
      span('layout', end - layout, end);
    };
    const wheel = (start, end) => {
      bench.pending.wheel += end - start;
    };
    const draw = (start, drawnAt) => {
      gl.finish();
      const end = performance.now();
      const submit = read(timings()).submitMs;
      const paint = drawnAt - start - submit;
      const gpu = submit + (end - drawnAt);
      span('paint', start, start + paint);
      span('gpu', start + paint, end);
      const { json, wheel: wheelMs, parse, layout } = bench.pending;
      const ms = end - bench.since;
      const frame = {
        at: end,
        ms,
        parse: json + parse,
        layout,
        paint: paint + wheelMs,
        gpu,
        other: Math.max(0, ms - (json + parse + layout + paint + wheelMs + gpu)),
      };
      bench.pending = { json: 0, wheel: 0, parse: 0, layout: 0 };
      bench.since = end;
      if (bench.recording) {
        const drawn = bench.stats();
        frame.boardsDrawn = drawn.boardsDrawn;
        frame.zoom = drawn.zoom;
        bench.frames.push(frame);
      }
      if (bench.firstBoard === null) {
        // Boards that are not read yet are drawn as placeholders, which have no
        // children, so a frame that painted more nodes than boards drew a board.
        const drawn = bench.stats();
        if (drawn && drawn.nodesPainted > drawn.boardsDrawn) {
          bench.firstBoard = end;
          bench.firstBoardInfo = drawn;
        }
      }
      return end;
    };
    const phases = {
      _omni_render_scene_load: load,
      _omni_render_scene_set_system: load,
      _omni_render_view_wheel: wheel,
      _omni_render_scene_draw: draw,
    };

    for (const name of Object.keys(module)) {
      if (!name.startsWith('_omni_render_') || unmeasured.has(name)) continue;
      const measured = phases[name] ?? ((start, end) => span('other', start, end));
      intercept(name, (call) => (...args) => {
        const start = performance.now();
        const result = call(...args);
        const returned = performance.now();
        const end = measured(start, returned) ?? returned;
        if (bench.recording) {
          const total = (bench.calls[name] ??= { calls: 0, ms: 0 });
          total.calls += 1;
          total.ms += end - start;
        }
        if (name === '_omni_render_scene_load') {
          bench.lastLoadEnd = end;
          bench.loads = (bench.loads ?? 0) + 1;
        }
        if (name === '_omni_render_scene_draw') bench.onFrame?.();
        return result;
      });
    }
  };

  // The scripted pan and zoom. `cfg` has the canvas bounds of the page's boards
  // (`bounds`), the board the page opens on (`opening`, the first of the page)
  // and the length of the path (`ms`), all in page pixels. The view is steered
  // with the same wheel events a person's trackpad makes: Ctrl and the wheel
  // zoom around the middle of the canvas, the wheel alone pans.
  const WHEEL_ZOOM_RATE = 0.002;
  // MIN_ZOOM and MAX_ZOOM of crates/omni-render/src/view.rs.
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
      const board = [opening.x + opening.width / 2, opening.y + opening.height / 2, fit(opening)];
      // As far out as the page fits, or the viewer's minimum zoom, which on the
      // Ordrar page is not enough: the page is too tall to have all its boards
      // in view (the frames record how many are). From there the pan goes from
      // one corner of the page to the opposite one, so that the view passes over
      // all of the page; along a side that the view is wider than, it keeps to
      // the middle.
      const out = fit(bounds);
      const across = (start, length, half) =>
        half * 2 >= length ? [start + length / 2, start + length / 2] : [start + half, start + length - half];
      const [left, right] = across(bounds.x, bounds.width, size.width / out / 2);
      const [top, bottom] = across(bounds.y, bounds.height, size.height / out / 2);
      // [time as a share of the path, x, y, zoom]: the opening board, as far out
      // as the viewer goes, the pan across the whole page, and in on the opening
      // board.
      const way = [
        [0, ...board],
        [0.2, left, top, out],
        [0.75, right, bottom, out],
        [1, ...board],
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
      bench.calls = {};
      const started = performance.now();
      bench.pending = { json: 0, wheel: 0, parse: 0, layout: 0 };
      bench.since = started;
      bench.recording = true;
      const end = () => {
        bench.onFrame = null;
        bench.recording = false;
        resolve({ frames: bench.frames, ticks: bench.ticks, calls: bench.calls });
      };
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
        // The last frame is drawn by the viewer's own loop, if the last step
        // moved the view at all.
        bench.onFrame = end;
        setTimeout(end, 1000);
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
