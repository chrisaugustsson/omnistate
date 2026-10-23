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
at least `id`, `name` and `index`, and sometimes `flows`, an object of
prototype flows, each `{id, name, startingFrame}` with the id of the board the
flow starts at. The converter reads the flows only to list them in the report. The archive does not list the pages in their design order, and the ids do
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
  layout. A child that fills an axis may shrink below its content in Penpot,
  whereas CSS keeps it at its content's minimum size, so the converter writes
  `minWidth` or `minHeight` 0 on every fill axis, unless the shape has a min of
  its own. `layoutItemAbsolute: true` takes a child out of the flow; it keeps a
  position relative to its parent. `layoutItemMinW` is seen; the other three
  bounds (`layoutItemMaxW`, `layoutItemMinH`, `layoutItemMaxH`) are ASSUMED.
- Text sizes by `growType`: `auto-width` hugs both ways, `auto-height` keeps the
  width and hugs the height, `fixed` keeps the box. Penpot rounds the boxes of
  its texts up to whole pixels, but a text that hugs an axis follows its content
  as the browser measures it, in a layout or not: the stored size is not kept as
  a `minWidth` or `minHeight`. The shapes after such a text move by the
  difference; `make penpot-compare` reports that as text drift and measures the
  structural mismatches with each text pinned to its stored box inside the
  comparison (docs/penpot-compare.md).
- `showContent: false` means the frame clips its children. Most frames say
  `true` or nothing, and those are left unclipped.

### Layouts

A frame with `layout: "flex"` has `layoutFlexDir` (`row`, `column`),
`layoutWrapType`, `layoutGap` (`rowGap`, `columnGap`), `layoutPadding` (`p1` top,
`p2` right, `p3` bottom, `p4` left), `layoutAlignItems`, `layoutJustifyContent`
and `layoutAlignContent`. Penpot's `stretch` for `layoutAlignItems` does not
stretch a child that hugs its content on the cross axis, whereas CSS does, so
the converter writes `start`; a child that fills the cross axis stretches
anyway. In every flex frame the `shapes` list runs in the
reverse of the visual order, for rows and columns alike, so the converter
reverses it. Children taken out of the flow are placed after the others.

A frame with `layout: "grid"` adds `layoutGridColumns` and `layoutGridRows`, lists
of `{type, value}` where `type` is `flex` (a share of the space), `auto` or
`fixed` (a length; ASSUMED, not seen), and `layoutGridCells`, an object of
cells with `column`, `row`, `columnSpan`, `rowSpan` (counted from 1) and the ids
of the `shapes` in them. Every grid seen aligns its items to the center
(`layoutAlignItems`) and start (`layoutJustifyItems`). The converter writes them
on each child as `alignSelf` and `justifySelf`. A cell's own `alignSelf` and
`justifySelf` (`auto` in every cell seen; other values are ASSUMED) win over the
grid's.

### Margins

`layoutItemMargin` is an object `{m1, m2, m3, m4}` for the top, right, bottom
and left margin, with `layoutItemMarginType` `simple` or `multiple`. A side
that is left out is 0, and a margin may be negative. The converter writes the
non-zero sides as the child's `margin`, for a child that flows in a layout.

### Looks

- `fills`: `{fillColor, fillOpacity}` is seen. `fillColorGradient` with `type`,
  `startX`, `startY`, `endX`, `endY` (fractions of the box) and `stops` (`color`,
  `opacity`, `offset`), and `fillImage` with the media `id` and
  `keepAspectRatio`, are ASSUMED. No shape seen has more than one fill, so the
  order of several is unknown; the converter keeps the list order.
- `strokes`: `strokeColor`, `strokeOpacity`, `strokeWidth`, `strokeStyle` (`solid`,
  `dashed`) and `strokeAlignment` (`inner`, `outer`; `center` is ASSUMED). A
  dashed stroke becomes a `dash` of the same length and gap, six stroke widths
  each (seen at a width of 2, 12 px of dash and 12 px of gap; other widths are
  ASSUMED to scale). `outer` and `center` become `align` `outside` and `center`
  on a frame or board; `inner` is the default. A vector's stroke is centered on
  its path whatever Penpot says.
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

`positionData` holds the lines of the text as the browser that last laid it out
put them: a list with one item per run per line, each with its `y`. The gap
between the `y` of the first two distinct lines is the line pitch the text was
laid out at. The browser rounded it, to whole pixels in `tirebud-idag.penpot`
(14 px at 1.45 gave 20, not 20.3) and to 1/64 px in `tirebud-ordrar.penpot`
(20.296875). The converter writes `lineHeight` as `(pitch + 1/128) / fontSize`,
unrounded, so that layout, which snaps a line to 1/64 px, gets the pitch back
exactly. A text of one line that hugs its content has no second line, and
its stored box height is the pitch. A text with no `positionData`, or a fixed
box of one line, keeps the multiple that Penpot stores.

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
masked groups (`maskedGroup`), blur, radial gradients, strokes that are neither
solid nor dashed, rotation, flips, mixed text styles, and text strokes and
shadows.

Prototypes and constraints are left out of v1 on purpose, but the shape around
them is converted, so each is listed with a property:

- `interactions` is a list on a shape (many shapes in the exports have an
  empty one). A shape with a non-empty list is listed once as having prototype
  links.
- `flows` of a page are listed once each, for the board they start at, with the
  name of the flow.
- `constraintsH` and `constraintsV` are `left` and `top` unless the designer
  changed them (`leftright`, `right`, `center`, `topbottom`, `bottom`, `scale`).
  A shape with any other value is listed once as having constraints.

### Shapes outside boards

Shapes that sit directly on a page without being a frame (texts, in both
exports, but also rects, groups and others) are converted into a board called
"Unplaced" on the same page, in their stacking order. The board sits at the top
left corner of their boxes and is as large as the boxes together, and each shape
keeps its place on the page. A page without such shapes has no Unplaced board.
The board has no Penpot id; it takes the id of its page. Penpot has no image of
it, so it gets the layout comparison only.

Component links are left out too. In `tirebud-ordrar.penpot` 10,005 shapes have
`componentRoot` true, all of them frames inside boards (375 are boards
themselves). Each such root names its component in `componentId`; a copy points
at the main shape through `shapeRef`, and the main instance has `mainInstance`
true. The shapes inside a root carry `shapeRef` (and sometimes `componentId`)
but no `componentRoot`. The converter draws a component's shapes as the plain
shapes they are and lists each root once, with property `componentId` and a
reason that names the component, so a report reader can see which parts of a
board come from components. The shapes inside a root get no entry of their own.

## Importing (`omni import penpot`)

`omni import penpot <file.penpot> --project <id> [--feature <name>]` (US-051,
US-105) imports an export made with "Export file + libraries" straight into the
project, with no change set. The design doc lands at the app level, or in the
UI pillar of the feature with `--feature`; the design system doc is always the
app level's. The reader and the
converter above do the work, so every rule of "What the converter takes from a
shape" applies (line pitch, filling children, hugging texts, the board Unplaced).
`omni_penpot::import::import_file` gives the whole import as data: `pages` (the
design doc), `design_system` (the design system doc), `attachments` and `report`.

An import into a project whose design system doc already has tokens or
components (a second import, or a design system an agent made) adds to its head,
because putting tokens or components replaces all of them. The command reads the
head's tokens and components first. Its tokens join the design system's, and a
token of the same path takes the export's value; a token where the design system
has a group of that path, or the other way round, is not imported and is in the
report. The modes are the design system's, then those of the export's themes
that it lacks (an export with one theme or none adds no mode); a token with a
value per mode takes its `$value` in a mode it has no value for, and the report
names it. Should the joined tree be refused, the design system's tokens stay as
they are and the report says why. The design system's components are sent first
with their IDs, so they and the instances that place them stay as they are, and
the export's components follow as new ones.
`omni_penpot::compare::compare_import` compares the imported boards with
Penpot's PNGs, in the mode of Penpot's active theme.

What the export holds, and what it becomes:

| Penpot | OmniState |
| --- | --- |
| Pages, in their `index` order | One `page` node each, named as the page, holding its boards |
| Boards and the shapes in them | As converted: boards, frames, texts, vectors and images |
| `components/*.json` | One `component` node each in the design system doc, converted from the main instance. Named `name`, or `path / name` when it has a path |
| Variants of one set (`variantId`, `variantProperties`) | Components that share the set's name and carry a `variant` property for each Penpot property, listing every value of the set, each with its own value as default |
| A copy of a component (`componentId`, `shapeRef`) | An `instance` of that component. An untouched copy has no overrides |
| A nested copy inside a component | An `instance` inside the component's tree |
| `touched` on a copy: `text-content-text`, `fill-group`, `visibility-group`, `swap-slot-<id>` | Overrides: `content`, `fills`, `visible`, `component`, keyed by the node of the component whose Penpot shape the layer copies (`shapeRef`) |
| `tokens.json` themes (`$themes`) | The modes of the token tree, named as the themes and in their order |
| Color, spacing and border radius tokens | `color`, `spacing` and `radius` tokens with their Penpot path. A token of a set that every theme enables has one value, a token of a set that one theme enables has that theme's value in its mode. `rgba(r, g, b, a)` becomes `#rrggbbaa`, a number becomes `16px`, and a reference to another token is followed and written out as its value |
| `appliedTokens` `fill` and `strokeColor` | The fill, the stroke and the text's color stay references: `$color.surface` |
| Library colors (`colors/`) | `color.library.<path>.<name>` tokens, the same in every mode |
| Library typographies (`typographies/`) | `typography.<path>.<name>` tokens |
| Images | Attachments; the design refers to them as `attachment:<id>` |
| Font files in `fonts/<family>/` beside the export, for each family a text uses | Attachments (`.ttf`, `.otf`), listed in `$extensions.omnistate.fontFiles` of every typography token of that family. Other files and unused families are not imported. The design viewer draws the text in them |

The main instances of components stay on their pages as the plain shapes they
are (a page of components keeps its look), and the component is converted from
the same shapes. A board that is itself a copy stays a board, because a page
holds only boards; its own copies inside are instances.

An instance holds only text, fills, visibility and swapped components, so what
else a copy changes on a layer (position or size, name, strokes, shadows, text
style, layout, a swapped component the design system cannot place) is listed
once per layer with the property `overrides`. A change to a layer inside a
nested instance cannot be keyed and is listed too.

Anything that cannot be imported is in the report, never dropped silently: the
entries of the converter, each with the page it is on, and entries without a
shape for tokens, colors and typographies. The tokens of a type with no group in
OmniState's tree (font sizes, weights and families, sizing, opacity and so on)
are listed by their Penpot path; a token that fails to resolve is listed with
the reason.

The command reads the export before it sends anything, so a file that is no
export leaves no doc behind. It uploads the images, starts the design system
doc unless the project has one, writes the tokens to its head, puts the
components in as often as it takes for the instances inside them to name
components the server has given IDs (the server assigns every node ID), starts
the design doc, and inserts the pages into its head: each page, then its boards
in batches of about a megabyte, and last it removes the page the new design
started with. `--json` prints `design_doc_id`, `design_system_doc_id` and the
whole `report`.
