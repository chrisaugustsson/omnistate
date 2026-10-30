// What crates/omni-render/tests/bench_ordrar.rs runs `make bench-ordrar`'s page
// and numbers with, without a browser: `openPage` loads the real
// render-sandbox/bench-ordrar-page.js with a clock only the test moves, a
// 1440x900 canvas that records its wheel events, and a renderer module whose
// calls take the time the test gives them. `run` builds what one run gives
// bench-ordrar-lib.mjs.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { frameProfile, frameStats, renderReport } from '../../../../render-sandbox/bench-ordrar-lib.mjs';

const PAGE_SCRIPT = new URL('../../../../render-sandbox/bench-ordrar-page.js', import.meta.url);

// The page of the bench's Ordrar path: 8118 by 329640 px, opening on a board that
// fills the canvas at zoom 1.
const ORDRAR = {
  bounds: { x: 0, y: 0, width: 8118, height: 329640 },
  opening: { x: 0, y: 0, width: 1440, height: 900 },
  ms: 10_000,
};

export function openPage() {
  let now = 0;
  let animationFrame = null;
  let timer = null;
  let played = null;
  let cost = 0;
  let finishMs = 0;
  let timings = { parseMs: 0, layoutMs: 0, paintMs: 0, submitMs: 0 };
  let stats = { boardsDrawn: 1, nodesPainted: 10, detail: 'full', zoom: 1 };
  const wheels = [];

  class HTMLCanvasElement {
    clientWidth = 1440;
    clientHeight = 900;
    getContext() {
      return {
        finish: () => {
          now += finishMs;
        },
      };
    }
    getBoundingClientRect() {
      return { left: 0, top: 0 };
    }
    dispatchEvent(event) {
      wheels.push({ at: now, ...event });
    }
  }
  class WheelEvent {
    constructor(type, init) {
      Object.assign(this, init);
    }
  }
  const window = {};
  vm.runInNewContext(readFileSync(PAGE_SCRIPT, 'utf8'), {
    window,
    performance: { now: () => now },
    PerformanceObserver: class {
      observe() {}
    },
    HTMLCanvasElement,
    WheelEvent,
    requestAnimationFrame: (callback) => {
      animationFrame = callback;
    },
    setTimeout: (callback) => {
      timer = callback;
    },
  });

  const takesTime = () => {
    now += cost;
    return true;
  };
  const module = {
    UTF8ToString: (text) => text,
    _omni_render_scene_stats: () => JSON.stringify(stats),
    _omni_render_scene_timings: () => JSON.stringify(timings),
    _omni_render_scene_load: takesTime,
    _omni_render_scene_set_system: takesTime,
    _omni_render_scene_draw: takesTime,
  };
  window.__benchHook(module);
  new HTMLCanvasElement().getContext('webgl2');

  const call = (name, ms, reported) => {
    cost = ms;
    timings = { ...timings, ...reported };
    module[name]();
  };
  const startPath = (cfg = ORDRAR) => {
    played = window.__bench.runPath(cfg);
  };
  const playTo = (tickMs) => {
    while (animationFrame) {
      const next = animationFrame;
      animationFrame = null;
      next();
      now += tickMs;
    }
    timer();
    return played;
  };
  return {
    bench: window.__bench,
    startPath,
    endPath: () => {
      now += ORDRAR.ms;
      return playTo(ORDRAR.ms);
    },
    stall: (ms) => {
      now += ms;
    },
    setSystem: ({ ms, layoutMs }) => call('_omni_render_scene_set_system', ms, { layoutMs }),
    sceneLoad: ({ ms, layoutMs }) => call('_omni_render_scene_load', ms, { layoutMs }),
    draw: ({ ms, submitMs = 0, finish = 0, drawn = stats }) => {
      finishMs = finish;
      stats = { ...stats, ...drawn };
      call('_omni_render_scene_draw', ms, { submitMs });
    },
    // Plays the whole path with an animation frame every `tickMs`, and returns
    // the wheel events the canvas got.
    flyPath: async (tickMs) => {
      startPath();
      await playTo(tickMs);
      return wheels;
    },
  };
}

export const frame = (ms, phases = {}) => ({ ms, zoom: 0.01, boardsDrawn: 350, parse: 0, layout: 0, paint: 0, gpu: 0, other: ms, ...phases });

// 18 fast frames and 2 slow ones: the median is `fast`, the 95th percentile `slow`.
export const framesOf = (fast, slow) => [...Array(18).fill(fast), slow, slow].map((ms) => frame(ms));

export const run = ({ firstBoardMs = 1000, frames = framesOf(12, 1300), peakMemoryMiB = 250, cold = {} } = {}) => ({
  firstBoardMs,
  firstBoardInfo: { boardsDrawn: 5, nodesPainted: 107 },
  cold,
  slowest: [],
  frames: frameStats(frames, []),
  frameProfile: frameProfile(frames),
  calls: [],
  peakMemoryMiB,
});

export const report = ({ runs, boards = 797, boardsOnPage = 769 }) =>
  renderReport({ machine: 'test machine', renderer: 'test renderer', apiImage: 'test builds', importMs: 1000, fileInfo: 'test file', boards, boardsOnPage, runs, date: '2026-10-30' });
