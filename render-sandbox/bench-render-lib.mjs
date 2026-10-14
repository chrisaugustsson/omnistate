// The numbers and verdicts of renderer gate A (US-008), apart from the browser:
// what the gate measures, how the five cold runs are reduced to medians, whether
// the medians are within the limits, and the Markdown report that goes to
// docs/renderer-gate.md. bench-render.mjs drives Chrome and calls into this
// module; the tests run it in Node.

// How the gate is run and what it must hold to. `boardMs` is the time from the
// data of the 2000-node board being loaded in the page to its first complete
// frame, `fileMs` the same for the 100-board stress file showing every board,
// and `frameP95Ms` the 95th-percentile frame time of the scripted zoom and pan
// path (16.7 ms is 60 frames per second).
export const config = {
  viewport: { width: 1440, height: 900 },
  deviceScaleFactor: 2,
  runs: 5,
  limits: { boardMs: 500, fileMs: 2000, frameP95Ms: 16.7 },
};

// What each measurement is called in the report, and its unit of comparison.
const MEASUREMENTS = [
  { name: 'boardMs', title: '2000-node board, data loaded to first complete frame', under: true },
  { name: 'fileMs', title: '100-board file, data loaded to first frame of every board', under: true },
  { name: 'frameP95Ms', title: 'Zoom and pan path, 95th-percentile frame time', under: false },
];

// The middle value of `values`, or the mean of the middle two. `values` is not
// reordered.
export function median(values) {
  if (values.length === 0) throw new Error('there are no values to take the median of');
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2 === 1 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

// The nearest-rank percentile: the value at position ceil(p / 100 * n), counting
// from 1, of the sorted values.
export function percentile(values, p) {
  if (values.length === 0) throw new Error('there are no values to take a percentile of');
  const sorted = [...values].sort((a, b) => a - b);
  // The small offset keeps products like 0.95 * 100 from rounding up a rank.
  const rank = Math.ceil((p * sorted.length) / 100 - 1e-9);
  return sorted[Math.min(sorted.length, Math.max(1, rank)) - 1];
}

// The 95th-percentile frame time of `frameTimes`.
export function frameP95(frameTimes) {
  return percentile(frameTimes, 95);
}

// Whether a WebGL renderer string names software rendering, which would measure
// the CPU and not the GPU the product is built for.
export function isSoftwareRenderer(renderer) {
  return /swiftshader|llvmpipe|softpipe|mesa offscreen|basic render|software/i.test(renderer);
}

// Throws an error that quotes `renderer` if it is software rendering.
export function assertHardwareRenderer(renderer) {
  if (isSoftwareRenderer(renderer)) {
    throw new Error(
      `the WebGL renderer is "${renderer}", which is software rendering: the gate must run on a GPU`,
    );
  }
}

// Reduces the runs, one { boardMs, fileMs, frameP95Ms } per cold page load, to
// the median of each measurement and checks each median against its limit.
export function evaluate(runs) {
  if (!Array.isArray(runs) || runs.length !== config.runs) {
    throw new Error(`the gate takes exactly ${config.runs} runs, not ${runs?.length}`);
  }
  const medians = {};
  const checks = [];
  for (const { name, under } of MEASUREMENTS) {
    const value = median(runs.map((run) => run[name]));
    const limit = config.limits[name];
    medians[name] = value;
    checks.push({ name, value, limit, pass: under ? value < limit : value <= limit });
  }
  return { medians, checks, pass: checks.every((check) => check.pass) };
}

function number(value) {
  return value.toFixed(1);
}

function verdict(pass) {
  return pass ? 'PASS' : 'FAIL';
}

// The Markdown of the Gate A section of docs/renderer-gate.md.
export function renderReport({ machine, renderer, runs, date }) {
  const result = evaluate(runs);
  const { width, height } = config.viewport;
  const lines = [
    '## Gate A: performance',
    '',
    `Result: **${verdict(result.pass)}**`,
    '',
    `- Date: ${date}`,
    `- Machine: ${machine}`,
    `- WebGL renderer: \`${renderer}\``,
    `- Chrome: headed, driven by Playwright, ${width}x${height} viewport, device pixel ratio ${config.deviceScaleFactor}`,
    `- Method: ${config.runs} runs, each measurement from a cold page load (a new browser, with an empty cache); the verdict is the median of the ${config.runs} runs`,
    '- Times are in milliseconds. A frame is complete when the GPU has finished it (`gl.finish()`).',
    '',
    '### Medians and limits',
    '',
    '| Measurement | Median | Limit | Verdict |',
    '| --- | --- | --- | --- |',
  ];
  for (const { name, title, under } of MEASUREMENTS) {
    const check = result.checks.find((candidate) => candidate.name === name);
    const limit = `${under ? 'under ' : ''}${check.limit}${under ? '' : ' or less'}`;
    lines.push(`| ${title} | ${number(check.value)} | ${limit} | ${verdict(check.pass)} |`);
  }
  lines.push('', `### The ${config.runs} runs`, '', '| Run | Board | File | Frame p95 |', '| --- | --- | --- | --- |');
  runs.forEach((run, index) => {
    lines.push(
      `| ${index + 1} | ${number(run.boardMs)} | ${number(run.fileMs)} | ${number(run.frameP95Ms)} |`,
    );
  });
  lines.push('');
  return lines.join('\n');
}
