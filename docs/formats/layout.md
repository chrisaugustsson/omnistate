# Layout

How the layout properties of a design map to CSS. The renderer lays designs
out by exactly these rules, with [Taffy](https://github.com/DioxusLabs/taffy)
(`crates/omni-render/src/layout.rs`), so a design lays out like the web page
built from it. US-009 exports designs as HTML with this CSS and checks that a
browser gives every node the same box.

Layout gives every node a box: `x`, `y`, `width` and `height` in CSS pixels,
in page coordinates, with the fractions of a pixel a browser keeps. No box is
ever stored in a design; layout computes them when a design is drawn.

The boxes are the ones Chrome gives at a device pixel ratio of 2, the ratio
that the renderer gate (US-008, US-009) compares them at. Chrome rounds the
heights of lines of text and their baselines to device pixels, which are half
a CSS pixel there (see [Text](#text)), so at other ratios its text boxes
differ by up to a pixel.

## Elements

Every node is one element (a `div`), and its children are the elements of its
children, in order. Every element is `box-sizing: border-box`, with no margin
and no border, so a node's `width` and `height` include its padding.

Boards and frames are `position: relative`, which makes each one the
containing block of its positioned children (see
[Placement](#placement-x-y-and-absolute)). A board or frame that is itself
positioned is `position: absolute` instead, which makes it one too.

A page is a canvas without a size. It is not laid out: its box is empty
(0 × 0) at x 0, y 0. Each board is laid out on its own and sits at its `x`
and `y` on the page, as `position: absolute; left: <x>px; top: <y>px`.
Nothing around a board limits its width, so a board that hugs its width is
`width: max-content`. A board has no parent to fill, so `fill` sizes a board
like `hug`.

## Layout modes

A board or frame states its `layout`, whose `mode` is one of:

| `mode` | CSS | Children |
| --- | --- | --- |
| `none` | `display: block` | Each sits at its `x` and `y` (`position: absolute`). |
| `flex` | `display: flex` | In a flexbox, by the [flex properties](#flex-layouts). |
| `grid` | `display: grid` | On a grid, by the [grid properties](#grid-layouts). |

A property that a layout leaves out keeps its CSS initial value.

## Flex layouts

| Property | CSS | Values (the initial value first) |
| --- | --- | --- |
| `direction` | `flex-direction` | `row`, `column`, `row-reverse`, `column-reverse` |
| `wrap` | `flex-wrap` | `nowrap`, `wrap`, `wrap-reverse` |
| `gap` | `row-gap` (`gap.row`) and `column-gap` (`gap.column`) | lengths, 0 when left out |
| `padding` | `padding-top`, `padding-right`, `padding-bottom`, `padding-left` | lengths, 0 when left out |
| `alignItems` | `align-items` | `normal` (left out), `start`, `center`, `end`, `stretch`, `baseline` |
| `justifyContent` | `justify-content` | `normal` (left out), `start`, `center`, `end`, `space-between`, `space-around`, `space-evenly` |

- The main axis runs along `direction`: horizontal for `row` and
  `row-reverse`, vertical for `column` and `column-reverse`. The cross axis
  runs across it. `column-gap` separates the children of a row and `row-gap`
  those of a column; with `wrap`, the other gap separates the lines.
- `start` and `end` are the CSS keywords `start` and `end`, not `flex-start`
  and `flex-end`: they follow the page's direction (left to right, top to
  bottom), whatever `direction` and `wrap` say. In a `row-reverse` row,
  `justifyContent: start` packs the children against the left edge, in
  reverse order. Left out, `justifyContent` packs them from where
  `direction` starts.
- `alignItems` left out is `normal`, which stretches like `stretch`.
- `align-content` keeps its initial value, `normal`, so wrapped lines share
  any free cross space equally.

## Grid layouts

| Property | CSS |
| --- | --- |
| `columns` | `grid-template-columns` |
| `rows` | `grid-template-rows` |
| `gap` | `row-gap` (`gap.row`) and `column-gap` (`gap.column`) |
| `padding` | `padding-top`, `padding-right`, `padding-bottom`, `padding-left` |

- A track is a length (`{"fixed": 240}` is `240px`), a share of the free
  space in the CSS `fr` unit (`{"fr": 1}` is `1fr`), or sized by its content
  (`"auto"` is `auto`). `"columns": [{"fixed": 100}, {"fr": 1}, "auto"]` is
  `grid-template-columns: 100px 1fr auto`.
- `columns` or `rows` left out is `none`: the grid makes tracks as it needs
  them. Such implicit tracks, and the rows that children placed beyond the
  last row add, are `auto` (`grid-auto-columns: auto`,
  `grid-auto-rows: auto`), and children without a line fill the grid row by
  row (`grid-auto-flow: row`).
- `justify-content` and `align-content` keep their initial value, `normal`,
  so `auto` tracks stretch into free space that no `fr` track takes.

### Grid placement

A child of a grid may set `gridColumn` and `gridRow`, which are `grid-column`
and `grid-row`. Each holds `start`, the line the child starts at, and `span`,
how many tracks it spans:

| Placement | CSS |
| --- | --- |
| `{"start": 2, "span": 3}` | `grid-column: 2 / span 3` |
| `{"start": 2}` | `grid-column: 2 / auto` (one track) |
| `{"span": 3}` | `grid-column: span 3 / auto` |
| `{}` or left out | `grid-column: auto / auto` |

Lines count from 1 at the start of the grid, or from -1 at its end: in a grid
of three columns, line -1 is line 4 and line -2 is line 3. A child without a
`start` goes into the next free cells. An absolute child is placed by its
`x` and `y` only, so it ignores `gridColumn` and `gridRow`.

## Sizing: `width` and `height`

Every node below a page sizes each axis with `width` and `height`: a fixed
length (`{"fixed": 320}`), `"hug"` or `"fill"`. An instance may leave an axis
out, which sizes it like `hug` until instances are resolved against their
components (US-032).

- `fixed` is that length: `width: 320px`.
- `hug` is `width: auto`: the node is as large as its
  [content](#content). Where CSS makes an `auto` width fit-content, a `hug`
  width is as wide as the content's widest arrangement (its max-content
  width), but no wider than the space available, unless the content's
  narrowest arrangement (its min-content width) is wider still. That is the
  case in a grid area, on the cross axis of a column that does not stretch
  the node, and for positioned nodes. Where nothing limits the width, as on
  a board or in a row that hugs its width, a `hug` width is the max-content
  width. A `hug` height is the content's height, whatever the space
  available: for a column that wraps, its longest line.
  [Flex content widths](#flex-content-widths) gives the widest and narrowest
  arrangements of flex layouts.
- `fill` is `width: auto` too, and the parent's layout makes the node as
  large as it can, by the declarations below.

The CSS that the parent's layout adds, per axis:

| Parent | Axis | `fixed` | `hug` | `fill` |
| --- | --- | --- | --- | --- |
| Flex layout | main axis | `flex-shrink: 0` | nothing (`flex: 0 1 auto`) | `flex-grow: 1; flex-basis: 0` |
| Flex layout | cross axis | nothing | nothing | `align-self: stretch` |
| Grid layout | horizontal | `justify-self: start` | `justify-self: start` | `justify-self: stretch` |
| Grid layout | vertical | `align-self: start` | `align-self: start` | `align-self: stretch` |
| `none` layout, or absolute | horizontal | nothing | nothing | `right: 0` |
| `none` layout, or absolute | vertical | nothing | nothing | `bottom: 0` |
| Page (a board) | horizontal | nothing | `width: max-content` | `width: max-content` |
| Page (a board) | vertical | nothing | nothing | nothing |

On the main axis of a flex layout:

- `fixed` keeps its length: it never grows or shrinks, even when the line
  overflows.
- `hug` starts at its content's max-content size and keeps CSS's initial
  `flex: 0 1 auto`: it never grows, and when the line overflows it shrinks
  like any CSS flex item, though never below its min-content size. A text
  that shrinks wraps.
- `fill` children share the free space of the line equally, whatever their
  content (`flex-basis: 0`), but never shrink below their min-content size,
  CSS's automatic minimum size of a flex item.

On the cross axis of a flex layout:

- `fixed` keeps its length, and `alignItems` places it.
- `hug` is as large as its content, and `alignItems` places it. `stretch`,
  and `normal` when `alignItems` is left out, stretch it across the line.
- `fill` stretches across the line, whatever `alignItems` says.

In a grid area, `fixed` and `hug` sit at the start (top left) of their area,
and `fill` fills it.

A positioned child (see [Placement](#placement-x-y-and-absolute)) that fills
an axis stretches from its `x` to the parent's right edge (from its `y` to
the bottom edge). One that hugs its width fits its content into the space
from its `x` to the parent's right edge: CSS's shrink-to-fit width of an
absolutely positioned box.

### Min and max sizes

`minWidth`, `maxWidth`, `minHeight` and `maxHeight` are `min-width`,
`max-width`, `min-height` and `max-height`. They bound every sizing,
including a stretched or filling one, and a min beats a max, as in CSS. A
fit-content width is bounded before the content is laid out in it, so a text
breaks its lines at its `maxWidth`.

Whatever its sizing and bounds, a node is never smaller than its padding:
stretched across a column narrower than its padding, a child is as wide as
its padding.

### Content

The content whose size a `hug` axis takes:

- A board or frame with a `flex` or `grid` layout: its children as its layout
  places them, plus its gaps and padding.
- A board or frame with a `none` layout: nothing, because positioned children
  take no space. A `none` frame that hugs is 0 px on that axis.
- A text: its lines (see [Text](#text)).
- An image, a vector or an instance: nothing in layout, so 0 px. Images are
  drawn into their box, and instances take their component's size in US-032.

### Flex content widths

The widest and narrowest arrangements of a flex layout's content are the ones
Chrome measures, plus the layout's padding:

- A row's widest arrangement puts all its children on one line, each at its
  max-content contribution, with the gaps between them. At its narrowest, a
  row that wraps puts each child on a line of its own, so it is as wide as
  its widest child at its narrowest. A row that does not wrap keeps its
  children on one line at their narrowest, with the gaps.
- A column is as wide as its widest child, and at its narrowest as its widest
  child at its narrowest. A column that wraps puts the lines its children
  wrap into side by side, with the gaps between them, each line as wide as
  its widest child. Its children wrap at its height when that is `fixed`, or
  when a row that does not wrap stretches it to the row's own `fixed`
  height, and otherwise at its `maxHeight`, or not at all.
- A child's contribution is its `fixed` width, or else the widest or
  narrowest arrangement of its own content, bounded by its `minWidth` and
  `maxWidth` and never narrower than its padding. A `fill` child counts at
  the widths of its content, as a `hug` child does, although it grows from a
  flex basis of 0 once its parent's width is known.

## Placement: `x`, `y` and `absolute`

The children of a `none` layout, and the children of flex and grid layouts
that set `"absolute": true`, are `position: absolute; left: <x>px;
top: <y>px`, with `x` and `y` 0 when left out. They are placed from their
parent's top-left corner, its edge rather than inside its padding, and they
take no space: their siblings and their parent's size ignore them. Children
that flow in a flex or grid layout have no `x` and `y`: their layout places
them.

## Visibility: `visible`

`"visible": false` is `display: none`. The node takes no space, and it and
every node below it get an empty box (0 × 0) at the top-left corner of its
parent.

## Text

A text node's element holds its `content` exactly as written, with
`white-space: pre-wrap`: spaces, tabs and line breaks are kept. When the text
is narrower than its content, its lines also break where Chrome breaks them
(`line-break: auto`, `word-break: normal`):

- After a run of spaces or tabs, never before one. The run stays at the end
  of its line, even where it does not fit (it hangs).
- Between two other ASCII characters, only after a hyphen (`-`) or a question
  mark, and before an opening bracket (`(`, `<`, `[`, `{`) that follows
  closing punctuation or most other symbols. Between a hyphen and a digit, a
  line breaks only when a letter or a digit comes before the hyphen, so `-1`
  stays whole. No other punctuation breaks, so `input/output`, URLs and
  `e.g.` stay whole.
- Next to any other character, where Unicode's line breaking algorithm
  (UAX #14) allows a break: before and after an em dash, after an en dash or
  an ellipsis, and between ideographs, for example. A no-break space
  (U+00A0) keeps the characters on both sides of it together, except that a
  line may break before one that follows a hyphen or a vertical bar (`|`),
  as UAX #14 allows. A hyphen or en dash that starts a word stays with it.

Each line takes as many pieces between these places as fit. Words never
break: a piece wider than its line overflows a line of its own
(`overflow-wrap: normal`).

| Property | CSS |
| --- | --- |
| `fontFamily` | `font-family: "<family>"`, or `font-family: "Inter"` when the renderer has no font files for the family |
| `fontSize` | `font-size: <size>px` |
| `fontWeight` | `font-weight` |
| `fontStyle` | `font-style` |
| `lineHeight` | `line-height: <multiple>` (unitless), or `line-height: normal` when left out |
| `letterSpacing` | `letter-spacing: <spacing>px` |

- The renderer bundles Inter Regular, and Inter stands in for every font
  family it lacks, so text measures the same everywhere. An export loads the
  same font files with `@font-face`. With Inter Regular the only face, every
  weight and style measures as Inter Regular.
- Text is shaped with Skia's paragraph layout: kerning pairs move closer, and
  `letterSpacing` adds space after every character, the last one included,
  as Chrome does. Like Chrome, the renderer shapes text with letter spacing
  without ligatures and contextual alternates (OpenType `liga`, `clig` and
  `calt`), so Inter's `->` keeps its two glyphs there instead of becoming an
  arrow.
- Lines are as tall as Chrome makes them at a device pixel ratio of 2. Every
  line is `lineHeight` × `fontSize` tall, rounded down to a 64th of a device
  pixel (1/128 px): 28.796875 px for 24 px text with `lineHeight` 1.2. With
  `line-height: normal`, a line is as tall as the font's ascent, descent and
  line gap, each rounded to whole device pixels: 19.5 px for 16 px Inter,
  whose ascent of 15.5 px and descent of 3.86 px round to 15.5 and 4 px. Half
  of the space between the line height and the glyphs goes above the glyphs,
  rounded down to a whole device pixel, which places the first baseline for
  `alignItems: baseline`.
- A tab moves its line on to the next tab stop, every eight spaces from the
  start of the line (CSS's initial `tab-size: 8`), letter spacing included,
  or to the stop after it when less than half a space is left.
- Width: a `fixed` or `fill` width, or a width that `alignItems` stretches,
  is the width the lines break at. A `hug` width is the text's fit-content
  width, where CSS makes it one: its max-content width is its widest line when
  lines break only at line breaks, the spaces at the end of the line
  included, and its min-content width is its widest piece between two places
  where a line may break, without the spaces after it.
- Height: the number of lines times the line height. An empty text has no
  line, and a line break at the very end of the content starts no line.
- `textAlign`, `textDecoration` and `color` do not change the box.

The renderer still breaks some text differently from Chrome: Thai, Lao,
Khmer and Myanmar break only at spaces (Chrome finds their words in a
dictionary), and a soft hyphen (U+00AD) breaks without the hyphen that Chrome
shows at the end of the line. It also measures the pieces of a line as the
whole line is shaped, where Chrome shapes a piece on its own when a ligature
or a contextual alternate spans the place where the line may break: without
letter spacing, the narrowest arrangement of `a->b` is narrower than in
Chrome, which draws the arrow when the text stays on one line but measures
`a-` and `>b` apart.

## Token references

Layout does not resolve token references yet; US-032 does. Until then, a
property whose value is a token reference counts as left out, so it keeps its
CSS initial value: a fixed size or a track is `auto`, a gap or padding is 0, a
min or max size is none, `lineHeight` is `normal`, `letterSpacing` is 0, a
font size is `medium` (16 px), a font weight is 400, and a font family is
Inter.

## Style

Fills, strokes, corner radii, shadows, opacity and `clip` never change a box.
`clip` is `overflow: clip`, which clips without changing layout, and strokes
take no space, so an export draws them without CSS borders.

## Example

```json
{
  "id": "brd00001", "type": "board", "name": "Card", "x": 100, "y": 40,
  "width": {"fixed": 320}, "height": "hug",
  "layout": {
    "mode": "flex", "direction": "column", "gap": {"row": 12},
    "padding": {"top": 16, "right": 16, "bottom": 16, "left": 16}
  },
  "children": [
    {
      "id": "txt00001", "type": "text", "name": "Title",
      "width": "fill", "height": "hug",
      "content": "Quarterly report", "fontFamily": "Inter", "fontSize": 20,
      "lineHeight": 1.2
    },
    {
      "id": "frm00001", "type": "frame", "name": "Badge",
      "absolute": true, "x": 280, "y": 8,
      "width": {"fixed": 24}, "height": {"fixed": 24},
      "layout": {"mode": "none"}, "children": []
    }
  ]
}
```

```html
<div style="position: absolute; left: 100px; top: 40px; box-sizing: border-box;
            width: 320px; display: flex; flex-direction: column; row-gap: 12px;
            padding: 16px">
  <div style="box-sizing: border-box; align-self: stretch; font-family: 'Inter';
              font-size: 20px; line-height: 1.2; white-space: pre-wrap">Quarterly report</div>
  <div style="position: absolute; left: 280px; top: 8px; box-sizing: border-box;
              width: 24px; height: 24px; display: block"></div>
</div>
```

The title fills the width of the column (`align-self: stretch`), so it is
288 px wide and breaks its lines there, and it hugs its height, the column's
main axis, with CSS's initial `flex: 0 1 auto`: one line of 24 px. The badge
sits 280 px from the card's left edge and 8 px from its top, and the card's
height ignores it: 16 + 24 + 16 = 56 px.
