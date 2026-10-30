// The numbers of `make bench-ordrar` (US-117), apart from the browser and the
// stack: how the main thread's time is split into the five phases, how the five
// runs are reduced, and the Markdown that goes to docs/bench-ordrar.md.
// bench-ordrar.mjs drives Chrome and Docker and calls into this module.
import { median, percentile } from './bench-render-lib.mjs';

export const config = {
  viewport: { width: 1440, height: 900 },
  deviceScaleFactor: 2,
  runs: 5,
  // The length of the scripted pan and zoom.
  pathMs: 10_000,
  // The limits of FR-28: first board under 2 s, 60 fps, the API under 512 MB.
  limits: { firstBoardMs: 2000, frameP95Ms: 16.7, peakMemoryMiB: 512 },
};

export const PHASES = ['network', 'parsing', 'layout', 'painting', 'gpu'];
export const PHASE_TITLES = {
  network: 'Network',
  parsing: 'Parsing',
  layout: 'Layout',
  painting: 'Painting',
  gpu: 'Uploading to the GPU',
  other: 'Other (JavaScript, React, other renderer calls, start-up, idle)',
};

// What the page names its spans, in the order a span wins where they overlap.
const SPAN_PHASE = { layout: 'layout', gpu: 'gpu', paint: 'painting', parse: 'parsing', other: 'other' };

// Splits the first `total` ms of a page load into the phases. `spans` are the
// intervals of the main thread that the page timed ({ phase: 'parse' | 'layout'
// | 'paint' | 'gpu' | 'other', start, end }); `requests` are the intervals ([start, end])
// of the network requests. Every instant counts once: for the main thread's
// phase if it was running one, else for the network if a request was open, else
// for `other`. Returns the ms of each phase.
export function splitLoad(total, spans, requests) {
  const clip = (value) => Math.min(total, Math.max(0, value));
  const cuts = new Set([0, total]);
  for (const { start, end } of spans) {
    cuts.add(clip(start));
    cuts.add(clip(end));
  }
  for (const [start, end] of requests) {
    cuts.add(clip(start));
    cuts.add(clip(end));
  }
  const edges = [...cuts].sort((a, b) => a - b);
  const result = { network: 0, parsing: 0, layout: 0, painting: 0, gpu: 0, other: 0 };
  const order = ['layout', 'gpu', 'paint', 'parse', 'other'];
  for (let i = 0; i + 1 < edges.length; i += 1) {
    const from = edges[i];
    const to = edges[i + 1];
    const middle = (from + to) / 2;
    const owner = order.find((name) =>
      spans.some((span) => span.phase === name && span.start <= middle && middle < span.end),
    );
    if (owner) result[SPAN_PHASE[owner]] += to - from;
    else if (requests.some(([start, end]) => start <= middle && middle < end)) result.network += to - from;
    else result.other += to - from;
  }
  return result;
}

const sum = (values) => values.reduce((total, value) => total + value, 0);
const mean = (values) => (values.length === 0 ? 0 : sum(values) / values.length);

// The frame times of one run: the median and 95th percentile of `frames` (each
// with `ms`, the time since the previous complete frame), their sum, the 95th
// percentile of the intervals between the page's animation frames (`ticks`),
// and how far out the path went: the lowest zoom and the most boards in view.
export function frameStats(frames, ticks) {
  const times = frames.map((frame) => frame.ms);
  const gaps = ticks.slice(1).map((tick, index) => tick - ticks[index]);
  return {
    count: frames.length,
    medianMs: median(times),
    p95Ms: percentile(times, 95),
    maxMs: Math.max(...times),
    totalMs: sum(times),
    intervalP95Ms: gaps.length > 0 ? percentile(gaps, 95) : 0,
    lowestZoom: Math.min(...frames.map((frame) => frame.zoom)),
    mostBoards: Math.max(...frames.map((frame) => frame.boardsDrawn)),
  };
}

// What a frame spends in each phase, as the mean over `frames`, and over the
// slowest 5% of them (the frames at or above the 95th percentile).
export function frameProfile(frames) {
  const cut = percentile(
    frames.map((frame) => frame.ms),
    95,
  );
  const keys = { parsing: 'parse', layout: 'layout', painting: 'paint', gpu: 'gpu', other: 'other' };
  const average = (list) => {
    const result = { total: mean(list.map((frame) => frame.ms)) };
    for (const [phase, key] of Object.entries(keys)) result[phase] = mean(list.map((frame) => frame[key]));
    return result;
  };
  return { typical: average(frames), slowest: average(frames.filter((frame) => frame.ms >= cut)) };
}

// Reduces the runs to the numbers `make bench-ordrar` prints: the median over
// the runs of the cold time to first board and of the frame times, and the
// highest memory peak of any run.
export function evaluate(runs) {
  if (!Array.isArray(runs) || runs.length !== config.runs) {
    throw new Error(`the bench takes exactly ${config.runs} runs, not ${runs?.length}`);
  }
  const medians = {
    firstBoardMs: median(runs.map((run) => run.firstBoardMs)),
    frameMedianMs: median(runs.map((run) => run.frames.medianMs)),
    frameP95Ms: median(runs.map((run) => run.frames.p95Ms)),
    peakMemoryMiB: Math.max(...runs.map((run) => run.peakMemoryMiB)),
    mostBoards: Math.max(...runs.map((run) => run.frames.mostBoards)),
  };
  const checks = [
    { name: 'firstBoardMs', value: medians.firstBoardMs, limit: config.limits.firstBoardMs, pass: medians.firstBoardMs < config.limits.firstBoardMs },
    { name: 'frameP95Ms', value: medians.frameP95Ms, limit: config.limits.frameP95Ms, pass: medians.frameP95Ms <= config.limits.frameP95Ms },
    { name: 'peakMemoryMiB', value: medians.peakMemoryMiB, limit: config.limits.peakMemoryMiB, pass: medians.peakMemoryMiB < config.limits.peakMemoryMiB },
  ];
  return { medians, checks, pass: checks.every((check) => check.pass) };
}

const number = (value) => value.toFixed(1);
const verdict = (pass) => (pass ? 'within the limit' : 'over the limit');

// The run whose value of `pick` is the median one (the lower of the middle two).
export function medianRun(runs, pick) {
  const sorted = [...runs].sort((a, b) => pick(a) - pick(b));
  return sorted[(sorted.length - 1) >> 1];
}

function profileRows(split, total, notApplicable = []) {
  const rows = [];
  for (const phase of [...PHASES, 'other']) {
    if (notApplicable.includes(phase)) {
      rows.push(`| ${PHASE_TITLES[phase]} | not applicable | not applicable |`);
    } else {
      const value = split[phase] ?? 0;
      rows.push(`| ${PHASE_TITLES[phase]} | ${number(value)} | ${total > 0 ? number((value / total) * 100) : '0.0'}% |`);
    }
  }
  return rows;
}

// The generated part of docs/bench-ordrar.md.
export function renderReport({ machine, renderer, apiImage, importMs, fileInfo, boards, boardsOnPage, runs, date }) {
  const result = evaluate(runs);
  const { width, height } = config.viewport;
  const m = result.medians;
  const coldRun = medianRun(runs, (run) => run.firstBoardMs);
  const frameRun = medianRun(runs, (run) => run.frames.p95Ms);
  const lowestZoom = Math.min(...runs.map((run) => run.frames.lowestZoom));
  const lines = [
    '## Baseline',
    '',
    `- Date: ${date}`,
    `- Machine: ${machine}`,
    `- WebGL renderer: \`${renderer}\``,
    `- Builds: ${apiImage}`,
    `- File: ${fileInfo}; imported in ${number(importMs / 1000)} s`,
    `- Chrome: headed, driven by Playwright, ${width}x${height} viewport, device pixel ratio ${config.deviceScaleFactor}`,
    `- Method: ${config.runs} runs, each from a new browser with an empty cache and a restarted API container; the numbers are the median of the ${config.runs} runs (memory: the highest peak). A frame is complete when the GPU has finished it (\`gl.finish()\`), and a frame's time is the time from the previous complete frame.`,
    `- Path: from the opening board out to zoom ${lowestZoom.toFixed(3)}, the viewer's minimum on this page, then across the whole page and back in. At that zoom at most ${m.mostBoards} of the ${boardsOnPage} boards of the page are in view; the page is too tall for all of them to be, and the file's other ${boards - boardsOnPage} boards are on other pages.`,
    '',
    '| Measurement | Result | FR-28 limit | |',
    '| --- | --- | --- | --- |',
    `| Cold time to first board | ${number(m.firstBoardMs)} ms | under ${config.limits.firstBoardMs} ms | ${verdict(result.checks[0].pass)} |`,
    `| Frame time over the ${config.pathMs / 1000} s pan and zoom, median | ${number(m.frameMedianMs)} ms | | |`,
    `| Frame time over the ${config.pathMs / 1000} s pan and zoom, 95th percentile | ${number(m.frameP95Ms)} ms | ${config.limits.frameP95Ms} ms or less (60 fps) | ${verdict(result.checks[1].pass)} |`,
    `| API peak memory | ${number(m.peakMemoryMiB)} MiB | under ${config.limits.peakMemoryMiB} MiB | ${verdict(result.checks[2].pass)} |`,
    '',
    '### The runs',
    '',
    '| Run | First board (ms) | Frames | Frame median (ms) | Frame p95 (ms) | Slowest frame (ms) | Animation frame interval p95 (ms) | Most boards in view | API peak (MiB) |',
    '| --- | --- | --- | --- | --- | --- | --- | --- | --- |',
  ];
  runs.forEach((run, index) => {
    const f = run.frames;
    lines.push(
      `| ${index + 1} | ${number(run.firstBoardMs)} | ${f.count} | ${number(f.medianMs)} | ${number(f.p95Ms)} | ${number(f.maxMs)} | ${number(f.intervalP95Ms)} | ${f.mostBoards} | ${number(run.peakMemoryMiB)} |`,
    );
  });

  const cold = coldRun.cold;
  lines.push(
    '',
    '## Profile',
    '',
    '### Cold load, navigation to the first board',
    '',
    `Run ${runs.indexOf(coldRun) + 1} of ${config.runs}, the median one: ${number(coldRun.firstBoardMs)} ms from the navigation to \`/docs/:doc\` to the first complete frame that draws a board that is read (${coldRun.firstBoardInfo.boardsDrawn} boards in view, ${coldRun.firstBoardInfo.nodesPainted} nodes painted). Every instant counts once: for the phase the main thread was in, then for other if it was in another renderer call or a long task, else for the network if a request was open, else for other.`,
    '',
    '| Phase | ms | Share of the load |',
    '| --- | --- | --- |',
    ...profileRows(cold, coldRun.firstBoardMs),
    '',
    'The requests of that load that took longest:',
    '',
    '| Request | Start (ms) | Duration (ms) | Transferred (KB) |',
    '| --- | --- | --- | --- |',
    ...coldRun.slowest.map(
      (request) => `| \`${request.name}\` | ${number(request.start)} | ${number(request.end - request.start)} | ${number(request.bytes / 1000)} |`,
    ),
  );

  const profile = frameRun.frameProfile;
  lines.push(
    '',
    '### Pan and zoom frames',
    '',
    `Run ${runs.indexOf(frameRun) + 1} of ${config.runs}, the one with the median 95th-percentile frame time: ${frameRun.frames.count} frames over ${number(frameRun.frames.totalMs)} ms. A frame is charged with everything since the previous frame, and its phases add up to its time: handling the wheel events (painting), the JSON that the page reads and writes (parsing), the renderer reading a page and a design system and laying them out (parsing and layout), the draw (painting) and the GPU finishing it. The rest is other: the page's JavaScript, the renderer calls that are in no phase, and the wait for the next animation frame. The network is not in a frame: boards are fetched between frames and show when they arrive.`,
    '',
    '| Phase | Typical frame, mean (ms) | Share | Slowest 5% of frames, mean (ms) | Share |',
    '| --- | --- | --- | --- | --- |',
  );
  for (const phase of [...PHASES, 'other']) {
    if (phase === 'network') {
      lines.push(`| ${PHASE_TITLES[phase]} | not applicable | not applicable | not applicable | not applicable |`);
      continue;
    }
    const a = profile.typical;
    const b = profile.slowest;
    lines.push(
      `| ${PHASE_TITLES[phase]} | ${number(a[phase])} | ${number((a[phase] / a.total) * 100)}% | ${number(b[phase])} | ${number((b[phase] / b.total) * 100)}% |`,
    );
  }
  lines.push(
    `| Frame | ${number(profile.typical.total)} | 100.0% | ${number(profile.slowest.total)} | 100.0% |`,
    '',
    'The renderer calls of that run that took longest:',
    '',
    '| Call | Calls | Total (ms) | Share of the path |',
    '| --- | --- | --- | --- |',
    ...frameRun.calls
      .slice(0, 6)
      .map((call) => `| \`${call.name}\` | ${call.calls} | ${number(call.ms)} | ${number((call.ms / frameRun.frames.totalMs) * 100)}% |`),
    '',
  );
  return lines.join('\n');
}
