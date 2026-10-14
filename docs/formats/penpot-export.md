# Penpot export format (`.penpot`)

What the `omni-penpot` crate reads from a Penpot file export, and what it does
with the rest. Everything here was observed in two exports made by hand from a
local Penpot 2.19 (`generatedBy` says `penpot/2.19.0-RC1`) with "treat assets as
basic objects" on. That option does not remove component links: the small export
has none, but the large one still marks component shapes (see "Left out, and
reported" below):

- `tirebud-idag.penpot`, 3.9 MB: one page, 2,426 shape files.
- `tirebud-ordrar.penpot`, 114 MB: five pages, 111,013 shape files.

The format is learned only from exported files and from Penpot's public user
documentation. Penpot's source code, including its schema definitions, is not
consulted (the licence rule of the project, see the PRD), so this document
describes what was seen, not what the format allows. Where the exports show no
case of something, the reader is lenient and the text below says so.

## The archive

A `.penpot` file is an ordinary zip archive (entries compressed with deflate)
holding one JSON file per object. The entries seen, with `<file>`, `<page>` and
`<shape>` standing for UUIDs:

| Entry | What it holds |
| --- | --- |
| `manifest.json` | The export's table of contents, read by the crate |
| `files/<file>.json` | Metadata of the file: name, version, features, options |
| `files/<file>/pages/<page>.json` | A page: its `id`, `name` and `index` |
| `files/<file>/pages/<page>/<shape>.json` | One shape of that page |
| `files/<file>/components/*.json`, `colors/*.json`, `typographies/*.json` | Library assets (only in the large export) |
| `files/<file>/tokens.json`, `tokens-status.json` | Design tokens |
| `files/<file>/thumbnails/...` | Small JSON records that point to thumbnail images |
| `objects/<id>.json` and `objects/<id>.png` | Thumbnail images with a JSON record each |

The crate reads `manifest.json`, the page files and the shape files. It ignores
the rest. The JSON is pretty-printed, and a `/` in a string is written `\/`.

### manifest.json

An object with `type` (`penpot/export-files`), `version` (1), `generatedBy`,
`referer`, `relations` (empty in both exports) and `files`, a list of the files
in the export. Each item has an `id`, a `name` and a list of `features`. Both
exports hold exactly one file. The crate takes the first item of `files` as the
file and reads its `id` and `name` from there. An archive without a
`manifest.json`, or a manifest that lists no file, is not an export and is
rejected with a message that names the manifest.

### Pages

The pages of a file are the entries named `files/<file>/pages/<page>.json`, where
the id sits directly after `pages/` and the name ends in `.json`. The file holds
at least `id`, `name` and `index`, and sometimes `flows` (prototype flows, not
read). The archive does not list the pages in their design order, and the ids do
not follow it either: the large export holds its five pages in an order that
matches neither, so the crate sorts them by `index`, ascending, with the id as a
tie breaker.

### Shapes and the root frame

The shapes of a page are the entries in the folder named after the page's id,
`files/<file>/pages/<page>/`, one per shape, named after the shape's `id`. They
are stored flat, not nested. The tree is in each shape's `shapes` field, an
ordered list of the ids of its children, bottom of the stacking order first.

Every page has one extra shape, the root frame, with the id
`00000000-0000-0000-0000-000000000000` (named "Root Frame", type `frame`, a
0.01 by 0.01 box). It is the parent of everything that sits directly on the
page, and its `shapes` list is the list of the page's top-level shapes, in
stacking order. The crate starts there and follows the `shapes` lists down. It
never goes by the order of the archive, and an entry that no list names is not
part of the tree. The root frame itself is not returned as a shape. A page
without a root frame entry is an error.

## What the crate takes from a shape

Each shape file is an object with many fields (colour, layout, text content,
effects and more) of which the reader uses seven:

| Field | Use |
| --- | --- |
| `id` | The shape's id. The entry name stands in if the field is missing. |
| `name` | The name shown in Penpot's layer list. Empty if missing. |
| `type` | The kind of shape, below. |
| `parentId` | The id of the parent shape. The id of the list that names the shape stands in if missing. |
| `selrect` | The box of the shape, in page coordinates: `x`, `y`, `width`, `height` and the corner coordinates `x1`, `y1`, `x2`, `y2`. |
| `shapes` | The children, as above. Only containers have it. |
| `hidden` | `true` for a shape the designer hid. Most shapes have no such field, and that means visible. |

### The box

The box of a shape comes from `selrect`, in the coordinates of the page (not
relative to the parent board). A shape also has top-level `x`, `y`, `width` and
`height` that normally say the same, but for every `path` in the exports they are
`null` and only `selrect` has the box. The reader therefore uses `selrect` first
and falls back on the top-level four, and gives no box if neither is complete.
Rotated shapes also carry `rotation`, `transform` and four `points`; the box
is the unrotated `selrect`, and the reader does not look at the rest.

### Shape types

The six values of `type` that the exports show, with the counts of the small
export:

- `frame`: a board or a frame inside one (982 shapes, the root frame included).
  Frames hold the layout fields (`layout`, `layoutFlexDir`, `layoutGap`,
  `layoutPadding` and a number of `layoutItem...` and `layoutAlign...` fields),
  corner radii `r1` to `r4`, `fills`, `strokes`, `shadow` and `opacity`. A frame
  that is a direct child of the root frame is a board.
- `group`: a plain group of shapes with no look of its own (87).
- `rect`: a rectangle (181).
- `circle`: an ellipse (84).
- `text`: a text, with its characters and styles in a `content` tree, plus
  `positionData`, the lines as Penpot's browser laid them out (798).
- `path`: a vector path, with the drawing in `content` and the SVG's own
  attributes in the `svg...` fields (294).

Any other `type` is read as an unknown shape. It keeps its id, its name, its
type string, its parent, its box and its children, and the shapes around it are
read as usual. Neither export holds one, so what such a shape looks like is not
observed; the crate's tests make up `bool` (a boolean shape of the Penpot user
guide), `image` and `svg-raw` shapes to check this. A shape that the tree names
but that has no entry, one whose JSON does not have the shape of a shape, and one
that a list names a second time (a loop) become unknown shapes with the types
`missing`, `unreadable` and `repeated`, without children.

Fields the reader does not use are skipped, so a field that Penpot adds later
does not break it.

## Boards

A board is a top-level `frame`: a direct child of the root frame whose `type` is
`frame`. The small export has 23 top-level shapes, of which 15 are boards and 8
are texts (labels); a top-level group, rect or unknown shape is not a board.

## What the converter takes from a shape

`omni-penpot convert` reads more of each shape than the reader does. Fields
below were seen in the two exports unless marked ASSUMED. ASSUMED means that
neither export holds an example, so the name follows Penpot's public naming and
the converter's tests build such shapes themselves; check it against a real
export when one turns up.

### Placement and size

- `selrect` gives the box in page coordinates. A child's `x` and `y` in the
  design are its box minus its parent's box. Boards keep the exact page
  position; every other number is rounded to a thousandth to hide float noise
  such as a width of 76.99999809265137.
- `layoutItemHSizing` and `layoutItemVSizing` take `fix`, `auto` (hug) and
  `fill`. They apply to a board and to children that flow in a flex or grid
  layout. `layoutItemAbsolute: true` takes a child out of the flow; it keeps a
  position relative to its parent. `layoutItemMinW` is seen; the other three
  bounds (`layoutItemMaxW`, `layoutItemMinH`, `layoutItemMaxH`) are ASSUMED.
- Text sizes by `growType`: `auto-width` hugs both ways, `auto-height` keeps the
  width and hugs the height, `fixed` keeps the box.
- `showContent: false` means the frame clips its children. Most frames say
  `true` or nothing, and those are left unclipped.

### Layouts

A frame with `layout: "flex"` has `layoutFlexDir` (`row`, `column`),
`layoutWrapType`, `layoutGap` (`rowGap`, `columnGap`), `layoutPadding` (`p1` top,
`p2` right, `p3` bottom, `p4` left), `layoutAlignItems`, `layoutJustifyContent`
and `layoutAlignContent`. In every flex frame the `shapes` list runs in the
reverse of the visual order, for rows and columns alike, so the converter
reverses it. Children taken out of the flow are placed after the others.

A frame with `layout: "grid"` adds `layoutGridColumns` and `layoutGridRows`, lists
of `{type, value}` where `type` is `flex` (a share of the space), `auto` or
`fixed` (a length; ASSUMED, not seen), and `layoutGridCells`, an object of
cells with `column`, `row`, `columnSpan`, `rowSpan` (counted from 1) and the ids
of the `shapes` in them. Every grid seen aligns its items to the center
(`layoutAlignItems`) and start (`layoutJustifyItems`), which OmniState cannot
express, so each such grid is listed in the report.

### Looks

- `fills`: `{fillColor, fillOpacity}` is seen. `fillColorGradient` with `type`,
  `startX`, `startY`, `endX`, `endY` (fractions of the box) and `stops` (`color`,
  `opacity`, `offset`), and `fillImage` with the media `id` and
  `keepAspectRatio`, are ASSUMED. No shape seen has more than one fill, so the
  order of several is unknown; the converter keeps the list order.
- `strokes`: `strokeColor`, `strokeOpacity`, `strokeWidth`, `strokeStyle` (`solid`,
  `dashed`) and `strokeAlignment` (`inner`, `outer`).
- `shadow`: a list of `{style, hidden, color: {color, opacity}, offsetX, offsetY,
  blur, spread}`. `drop-shadow` is seen, `inner-shadow` is ASSUMED. `blur` on a
  shape (`type`, `value`, `hidden`) is ASSUMED.
- `r1` to `r4` are the corner radii. All four were equal everywhere, so the
  corner order (top left, top right, bottom right, bottom left) is ASSUMED.
- `opacity`, `hidden`, `rotation`, `flipX`, `flipY`. A rotated or flipped shape
  is listed in the report and drawn upright.

### Text

`content` is a tree of `root`, `paragraph-set`, `paragraph` and runs holding
`text`. Style numbers are strings (`"18"`, `"1.2"`). A run carries `fontFamily`,
`fontSize`, `fontWeight`, `fontStyle`, `lineHeight` (a multiple of the size),
`letterSpacing`, `textAlign`, `textDecoration`, `textTransform` and `fills`.
Paragraphs are joined with a newline, and `textTransform` is applied to the
characters. A text whose runs differ in style is listed as mixed and takes the
style of its first run. The decorations `underline` and `line-through` are
ASSUMED. The shape's own `fills` are empty for text.

### Paths and circles

A `path`'s `content` is an SVG path string in absolute page coordinates, and
`selrect` is its box; the converter subtracts the box origin from every absolute
coordinate. A `circle` becomes an ellipse path in its box.

### Images

ASSUMED: an `image` shape has `metadata.id`, and a `fillImage` has `id`; both name
a media record `files/<file>/media/<id>.json` whose `mediaId` names the stored
file `objects/<mediaId>.<extension>`. The image is written as an attachment
named after the media id.

### Left out, and reported

Shapes of type `bool` and any unknown type (with the shapes inside them),
shapes that sit directly on the page outside every board, masked groups
(`maskedGroup`), blur, radial gradients, dashed strokes, stroke alignments other
than `inner`, item margins (`layoutItemMargin`), rotation, flips, grid item
alignment, mixed text styles, and text strokes and shadows.

Component links are left out too. In `tirebud-ordrar.penpot` 10,005 shapes have
`componentRoot` true, all of them frames inside boards (375 are boards
themselves). Each such root names its component in `componentId`; a copy points
at the main shape through `shapeRef`, and the main instance has `mainInstance`
true. The shapes inside a root carry `shapeRef` (and sometimes `componentId`)
but no `componentRoot`. The converter draws a component's shapes as the plain
shapes they are and lists each root once, with property `componentId` and a
reason that names the component, so a report reader can see which parts of a
board come from components. The shapes inside a root get no entry of their own.
