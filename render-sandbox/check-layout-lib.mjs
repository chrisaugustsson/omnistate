// What renderer gate B (US-009) does apart from the browser: the HTML that
// Chrome renders, the comparison of Chrome's boxes with the renderer's, and the
// Markdown report that goes to docs/renderer-gate.md. check-layout.mjs drives
// Chrome and calls into this module; the tests run it in Node.
import { config as gateA } from './bench-render-lib.mjs';
import { bundledFonts, exportHtml } from './layout-export.mjs';

// The viewport and the device pixel ratio are gate A's, so that both gates
// measure the same page; `tolerancePx` is how far apart a box may be from
// Chrome's in x, y, width or height.
export const config = {
  viewport: gateA.viewport,
  deviceScaleFactor: gateA.deviceScaleFactor,
  tolerancePx: 2,
};

const PROPERTIES = ['x', 'y', 'width', 'height'];

/**
 * The HTML page that Chrome renders for `design`, a page of boards or a board:
 * the export of `make chrome-boxes`, with the bundled fonts loaded through
 * `@font-face`.
 */
export function prepareExport(design) {
  return { html: exportHtml(design, bundledFonts()) };
}

// At most ten of `ids`, and how many more there are.
function named(ids) {
  const shown = ids.slice(0, 10).join(', ');
  return ids.length > 10 ? `${shown} and ${ids.length - 10} more` : shown;
}

/**
 * Compares the boxes that Chrome gives the nodes of a design with the
 * renderer's, both `{ x, y, width, height }` by node ID. Returns the number of
 * nodes `compared`, the `mismatches` whose largest difference of x, y, width or
 * height is more than `tolerancePx`, the largest difference first, the `largest`
 * difference of all, within the tolerance or not, and whether the boxes `pass`.
 * Throws an error that names the nodes when the two do not hold the same ones.
 */
export function compareBoxes(chrome, renderer, tolerancePx = config.tolerancePx) {
  const ids = Object.keys(chrome);
  const onlyChrome = ids.filter((id) => !(id in renderer));
  const onlyRenderer = Object.keys(renderer).filter((id) => !(id in chrome));
  if (onlyChrome.length > 0 || onlyRenderer.length > 0) {
    const parts = [];
    if (onlyChrome.length > 0) parts.push(`only Chrome has a box for ${named(onlyChrome)}`);
    if (onlyRenderer.length > 0) parts.push(`only the renderer has a box for ${named(onlyRenderer)}`);
    throw new Error(`the boxes are not of the same nodes: ${parts.join(', and ')}`);
  }
  const mismatches = [];
  const largest = { id: ids[0] ?? null, property: PROPERTIES[0], difference: 0 };
  for (const id of ids) {
    let worst = { property: PROPERTIES[0], difference: 0 };
    for (const property of PROPERTIES) {
      const difference = Math.abs(chrome[id][property] - renderer[id][property]);
      if (difference > worst.difference) worst = { property, difference };
    }
    if (worst.difference > largest.difference) Object.assign(largest, { id }, worst);
    if (worst.difference > tolerancePx) {
      mismatches.push({ id, ...worst, chrome: chrome[id], renderer: renderer[id] });
    }
  }
  mismatches.sort((a, b) => b.difference - a.difference);
  return { compared: ids.length, mismatches, largest, pass: mismatches.length === 0 };
}

// Plain numbers: no exponent, no thousands separators, no trailing zeros.
const number = (value) => String(Number(value.toFixed(3)));

const box = ({ x, y, width, height }) => `${number(x)}, ${number(y)}, ${number(width)} x ${number(height)}`;

/** The Markdown of the Gate B section of docs/renderer-gate.md. */
export function renderReport({ machine, chrome, date, comparison }) {
  const { width, height } = config.viewport;
  const { compared, mismatches, largest, pass } = comparison;
  const lines = [
    '## Gate B: layout parity',
    '',
    `Result: **${pass ? 'PASS' : 'FAIL'}**`,
    '',
    `- Date: ${date}`,
    `- Machine: ${machine}`,
    `- Chrome: ${chrome}, headed, driven by Playwright, ${width}x${height} viewport, device pixel ratio ${config.deviceScaleFactor}`,
    '- Compared: every node of the stress file, the box that `getBoundingClientRect` gives in Chrome against the box that the WebAssembly build of the renderer lays out in the page',
    '- Chrome renders the export of `make chrome-boxes`: line heights in px, never `normal`, and the bundled Inter loaded through `@font-face`',
    `- Tolerance: ${config.tolerancePx} px, for each of x, y, width and height`,
    `- Nodes compared: ${compared}`,
    `- Mismatches: ${mismatches.length}`,
    largest.difference > 0
      ? `- Largest difference: ${number(largest.difference)} px, the ${largest.property} of node ${largest.id}`
      : '- Largest difference: 0 px, every box is equal',
    pass
      ? `- Known differences of docs/formats/layout.md: no node of the stress file is beyond ${config.tolerancePx} px, so each is either fixed or stays within ${config.tolerancePx} px on the stress file`
      : '- Known differences of docs/formats/layout.md: not confirmed, because nodes are beyond the tolerance (below)',
    '',
  ];
  if (mismatches.length > 0) {
    lines.push('### Mismatches', '', '| Node | Property | Difference | Chrome (x, y, size) | Renderer (x, y, size) |', '| --- | --- | --- | --- | --- |');
    for (const mismatch of mismatches) {
      lines.push(
        `| ${mismatch.id} | ${mismatch.property} | ${number(mismatch.difference)} | ${box(mismatch.chrome)} | ${box(mismatch.renderer)} |`,
      );
    }
    lines.push('');
  }
  return lines.join('\n');
}

// What names a section: its heading up to the first colon, `## Gate B`.
const headingKey = (line) => line.split(':')[0].trim();

/**
 * `document` with the section that has the heading of `section` (compared up to
 * the first colon) replaced by it, every other section as it was, or `section`
 * added after the others when `document` has none like it.
 */
export function spliceReport(document, section) {
  const key = headingKey(section.split('\n')[0]);
  const lines = document.split('\n');
  const start = lines.findIndex((line) => line.startsWith('## ') && headingKey(line) === key);
  if (start < 0) return `${document.trimEnd()}\n\n${section.trimEnd()}\n`;
  let end = lines.findIndex((line, index) => index > start && line.startsWith('## '));
  if (end < 0) end = lines.length;
  const before = lines.slice(0, start).join('\n');
  const after = lines.slice(end).join('\n');
  return `${before}${before ? '\n' : ''}${section.trimEnd()}\n${after ? `\n${after}` : ''}`;
}
