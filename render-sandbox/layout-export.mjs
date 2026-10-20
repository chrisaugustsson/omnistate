// Exports a design as HTML and CSS by the rules of docs/formats/layout.md, so
// that a browser lays it out the way the renderer does. `make chrome-boxes`
// renders the export in headless Chrome (chrome-boxes.mjs), and the layout
// parity check of the renderer gate renders it in headed Chrome.
//
// The export holds layout and nothing else: fills, strokes (dashed, inside,
// centered or outside the edge), corner radii, shadows, opacity and the color
// of text never change a box, so they are left out, and `clip` is `overflow: clip`, which clips without changing layout.
import { readFileSync } from 'node:fs';

// The bundled fonts, which the renderer compiles in: the export loads the same
// files through `@font-face`, so text measures the same in both.
const FONT_FILES = [
  { family: 'Inter', file: new URL('../crates/omni-render/fonts/Inter-Regular.ttf', import.meta.url) },
];

// The family that stands in for every family without font files.
const FALLBACK_FAMILY = 'Inter';

// CSS's initial `font-size`, `medium`, which a token reference stands for.
const MEDIUM_FONT_SIZE = 16;

// Line heights are snapped to multiples of 1/64 px, which Chrome lays out in
// at every integer device pixel ratio.
const LINE_HEIGHT_UNITS_PER_PX = 64;

/** Reads the bundled font files and their vertical metrics. */
export function bundledFonts() {
  return FONT_FILES.map(({ family, file }) => {
    const data = readFileSync(file);
    return { family, data, ...verticalMetrics(data) };
  });
}

// The normal line height of a font, as a multiple of the font size: its
// ascent, descent and line gap from the `hhea` table, which is where the
// renderer's Skia reads them. For Inter it is (1984 + 494 + 0) / 2048.
function verticalMetrics(data) {
  const tables = new Map();
  for (let index = 0; index < data.readUInt16BE(4); index += 1) {
    const entry = 12 + index * 16;
    tables.set(data.toString('latin1', entry, entry + 4), data.readUInt32BE(entry + 8));
  }
  const head = tables.get('head');
  const hhea = tables.get('hhea');
  if (head === undefined || hhea === undefined) {
    throw new Error('a bundled font has no head or hhea table');
  }
  const unitsPerEm = data.readUInt16BE(head + 18);
  const ascent = data.readInt16BE(hhea + 4);
  const descent = -data.readInt16BE(hhea + 6);
  const lineGap = data.readInt16BE(hhea + 8);
  return { normalLineHeight: (ascent + descent + lineGap) / unitsPerEm };
}

/**
 * The HTML page of `design`, a page of boards or a single board, with the
 * `fonts` of `bundledFonts()` loaded through `@font-face`. Every node is a
 * `div` with its ID in `data-node`.
 */
export function exportHtml(design, fonts) {
  const context = { fonts: new Map(fonts.map((font) => [font.family, font])) };
  const roots = design.type === 'page' ? design.children ?? [] : [design];
  const faces = fonts.flatMap(({ family, data }) => {
    const source = `url(data:font/ttf;base64,${data.toString('base64')}) format("truetype")`;
    // One face for every weight and one for italics, so Chrome never
    // synthesizes a bold or an oblique from the only file there is.
    return ['normal', 'italic'].map(
      (style) =>
        `@font-face{font-family:"${family}";src:${source};font-weight:1 1000;font-style:${style}}`,
    );
  });
  return [
    '<!doctype html>',
    '<html><head><meta charset="utf-8">',
    `<style>${faces.join('')}html,body{margin:0;padding:0}`
      + '[data-node]{box-sizing:border-box;margin:0;border:0}</style>',
    '</head><body>',
    roots.map((root) => element(root, ROOT, context)).join(''),
    '</body></html>',
    '',
  ].join('');
}

/**
 * The box of every node of `design`, `{x, y, width, height}` by node ID, from
 * `rects`, the `[x, y, width, height]` that the browser gives each node's
 * element in page coordinates. A page is a canvas without a size, so its box
 * is empty at its origin. A hidden node and every node below it have no
 * element boxes: they get an empty box at the top-left corner of their parent,
 * and a hidden board, whose parent is the page, at its own `x` and `y`.
 */
export function boxesOf(design, rects) {
  const boxes = {};
  const collect = (node, parent, hidden) => {
    const shown = !hidden && node.visible !== false;
    const [x, y, width, height] = shown ? rects[node.id] : [parent.x, parent.y, 0, 0];
    boxes[node.id] = { x, y, width, height };
    for (const child of node.children ?? []) {
      collect(child, boxes[node.id], !shown);
    }
  };
  if (design.type === 'page') {
    boxes[design.id] = { x: 0, y: 0, width: 0, height: 0 };
  }
  for (const root of design.type === 'page' ? design.children ?? [] : [design]) {
    collect(root, { x: root.x ?? 0, y: root.y ?? 0 }, false);
  }
  return boxes;
}

// How a parent places a child.
const ROOT = { kind: 'root' };
const POSITIONED = { kind: 'positioned' };
const GRID = { kind: 'grid' };
const flexFlow = (row) => ({ kind: 'flex', row });

// A length that is a number. A token reference, which layout does not resolve
// yet, counts as left out, so the property keeps its CSS initial value.
const literal = (value) => (typeof value === 'number' ? value : undefined);

// A node's sizing of one axis: 'hug', 'fill', `{ fixed: <px or undefined> }`.
// An instance may leave an axis out, which sizes it like `hug`.
function sizing(value) {
  if (value === 'fill') return 'fill';
  if (value !== null && typeof value === 'object') return { fixed: literal(value.fixed) };
  return 'hug';
}

const px = (length) => `${length}px`;

function element(node, flow, context) {
  const declarations = [];
  const width = sizing(node.width);
  const height = sizing(node.height);
  placement(declarations, node, flow, width, height);
  sizeBounds(declarations, node);
  let children = [];
  let content = '';
  switch (node.type) {
    case 'board':
    case 'frame': {
      // The containing block of the children that sit at their `x` and `y`.
      if (flow.kind !== 'root' && flow.kind !== 'positioned') {
        declarations.push('position:relative');
      }
      const childFlow = layout(declarations, node.layout);
      if (node.clip === true) declarations.push('overflow:clip');
      children = (node.children ?? []).map((child) => {
        const positioned = child.absolute === true && childFlow !== POSITIONED;
        return element(child, positioned ? POSITIONED : childFlow, context);
      });
      break;
    }
    case 'text':
      textStyle(declarations, node, context);
      content = escapeText(node.content ?? '');
      break;
    case 'image':
    case 'vector':
    case 'instance':
      // No content that takes space: an empty box, whose size is its own.
      break;
    default:
      throw new Error(`node ${node.id} has the type ${JSON.stringify(node.type)}, which cannot be laid out`);
  }
  // A hidden node takes no space.
  if (node.visible === false) declarations.push('display:none');
  return `<div data-node="${node.id}" style="${declarations.join(';')}">${children.join('')}${content}</div>`;
}

// The CSS of a node's size on each axis and of how its parent places it, by
// the tables of docs/formats/layout.md.
function placement(declarations, node, flow, width, height) {
  const isFixed = (size) => typeof size === 'object' && size.fixed !== undefined;
  const fixedSize = (property, size) => {
    if (isFixed(size)) declarations.push(`${property}:${px(size.fixed)}`);
  };
  // A margin sits outside the node's box in whatever places it. A board sits
  // on its page by its `x` and `y`, so its margin is left out.
  if (flow.kind !== 'root') {
    for (const side of ['top', 'right', 'bottom', 'left']) {
      const length = literal(node.margin?.[side]);
      if (length !== undefined) declarations.push(`margin-${side}:${px(length)}`);
    }
  }
  switch (flow.kind) {
    case 'root':
      // A board sits at its `x` and `y` on its page. Nothing around it limits
      // its width, so one that hugs or fills it is as wide as its content.
      declarations.push('position:absolute', `left:${px(node.x ?? 0)}`, `top:${px(node.y ?? 0)}`);
      if (isFixed(width)) fixedSize('width', width);
      else declarations.push('width:max-content');
      fixedSize('height', height);
      break;
    case 'positioned':
      declarations.push('position:absolute', `left:${px(node.x ?? 0)}`, `top:${px(node.y ?? 0)}`);
      fixedSize('width', width);
      fixedSize('height', height);
      // A node that fills an axis stretches to the parent's far edge.
      if (width === 'fill') declarations.push('right:0');
      if (height === 'fill') declarations.push('bottom:0');
      break;
    case 'flex': {
      const [main, cross] = flow.row ? [width, height] : [height, width];
      fixedSize('width', width);
      fixedSize('height', height);
      if (main === 'fill') declarations.push('flex-grow:1', 'flex-basis:0');
      else if (typeof main === 'object') declarations.push('flex-shrink:0');
      if (cross === 'fill') declarations.push('align-self:stretch');
      break;
    }
    case 'grid':
      fixedSize('width', width);
      fixedSize('height', height);
      // An item that fills its area stretches to it, and any other sits at the
      // start of its area, unless it aligns itself.
      declarations.push(`justify-self:${node.justifySelf ?? (width === 'fill' ? 'stretch' : 'start')}`);
      declarations.push(`align-self:${node.alignSelf ?? (height === 'fill' ? 'stretch' : 'start')}`);
      if (node.gridColumn !== undefined) declarations.push(`grid-column:${gridLines(node.gridColumn)}`);
      if (node.gridRow !== undefined) declarations.push(`grid-row:${gridLines(node.gridRow)}`);
      break;
    default:
      throw new Error(`unknown flow ${flow.kind}`);
  }
}

function sizeBounds(declarations, node) {
  for (const [key, property] of [
    ['minWidth', 'min-width'],
    ['maxWidth', 'max-width'],
    ['minHeight', 'min-height'],
    ['maxHeight', 'max-height'],
  ]) {
    const length = literal(node[key]);
    if (length !== undefined) declarations.push(`${property}:${px(length)}`);
  }
}

// `grid-column` or `grid-row`: `<start> / span <span>`, where a start or a
// span that is left out is `auto`.
function gridLines({ start, span }) {
  if (start !== undefined) return `${start} / ${span === undefined ? 'auto' : `span ${span}`}`;
  return `${span === undefined ? 'auto' : `span ${span}`} / auto`;
}

// The CSS of a board's or frame's layout, and how it places its children.
function layout(declarations, { mode, ...properties }) {
  const lengths = (entries) => {
    for (const [property, value] of entries) {
      const length = literal(value);
      if (length !== undefined) declarations.push(`${property}:${px(length)}`);
    }
  };
  const spacing = () => {
    lengths([
      ['row-gap', properties.gap?.row],
      ['column-gap', properties.gap?.column],
      ['padding-top', properties.padding?.top],
      ['padding-right', properties.padding?.right],
      ['padding-bottom', properties.padding?.bottom],
      ['padding-left', properties.padding?.left],
    ]);
  };
  switch (mode) {
    case 'none':
      declarations.push('display:block');
      return POSITIONED;
    case 'flex': {
      declarations.push('display:flex');
      // A property that is left out keeps its CSS initial value. `start` and
      // `end` are the keywords of those names, not `flex-start` and `flex-end`.
      for (const [property, value] of [
        ['flex-direction', properties.direction],
        ['flex-wrap', properties.wrap],
        ['align-items', properties.alignItems],
        ['justify-content', properties.justifyContent],
      ]) {
        if (value !== undefined) declarations.push(`${property}:${value}`);
      }
      spacing();
      const direction = properties.direction ?? 'row';
      return flexFlow(direction === 'row' || direction === 'row-reverse');
    }
    case 'grid':
      declarations.push('display:grid');
      for (const [property, tracks] of [
        ['grid-template-columns', properties.columns],
        ['grid-template-rows', properties.rows],
      ]) {
        if (tracks?.length > 0) declarations.push(`${property}:${tracks.map(track).join(' ')}`);
      }
      spacing();
      return GRID;
    default:
      throw new Error(`unknown layout mode ${JSON.stringify(mode)}`);
  }
}

// A grid track: a length, a share of the free space or `auto`.
function track(track) {
  if (track === 'auto') return 'auto';
  if (track.fr !== undefined) return `${track.fr}fr`;
  const length = literal(track.fixed);
  return length === undefined ? 'auto' : px(length);
}

// The CSS of a text's font, and of the lines that break at its width.
function textStyle(declarations, node, context) {
  // A token reference for the family counts as left out.
  const named = typeof node.fontFamily === 'string' && !node.fontFamily.startsWith('$');
  const font = context.fonts.get(named ? node.fontFamily : undefined) ?? context.fonts.get(FALLBACK_FAMILY);
  const size = literal(node.fontSize) ?? MEDIUM_FONT_SIZE;
  declarations.push('white-space:pre-wrap', `font-family:'${font.family}'`, `font-size:${px(size)}`);
  const weight = literal(node.fontWeight);
  if (weight !== undefined) declarations.push(`font-weight:${weight}`);
  if (node.fontStyle === 'italic') declarations.push('font-style:italic');
  const spacing = literal(node.letterSpacing);
  if (spacing !== undefined) declarations.push(`letter-spacing:${px(spacing)}`);
  // Every line height is explicit, in px: a multiple of the font size, or the
  // font's normal line height at full precision, snapped to 1/64 px, so Chrome
  // gives the same line heights at any pixel ratio and never rounds `normal`.
  const multiple = literal(node.lineHeight) ?? font.normalLineHeight;
  const height = Math.floor(size * multiple * LINE_HEIGHT_UNITS_PER_PX) / LINE_HEIGHT_UNITS_PER_PX;
  declarations.push(`line-height:${px(height)}`);
}

const escapeText = (text) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
