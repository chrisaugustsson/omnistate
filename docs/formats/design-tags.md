# Design tags

Agents read and write designs as short, JSX-like tags (Q26, FR-33). Storage
stays JSON (the node model of `crates/omni-core/src/design/`), and the parser
and printer in `omni_core::design::tags` convert between the two without loss:
printing a node and parsing it back gives the same node, to the last digit of
every number.

```
<Frame id="k3f9x2qa" name="Card" dir="col" gap="$space.md" pad={16} fill="$color.surface">
  <Text id="m4n5p6q7" name="Title" content="Din varukorg" fontFamily="Inter" fontSize={24}/>
  <Frame name="Actions" dir="row" gap={8}/>
</Frame>
```

## Tags and attributes

One tag is one node, and its children are the node's children, in order. A tag
without children may close itself: `<Frame name="Card"/>`. Each tag is named
after the type of its node.

| Tag | Node type | Children |
| --- | --- | --- |
| `<Page>` | `page` | boards |
| `<Board>` | `board` | any node except boards and pages |
| `<Frame>` | `frame` | any node except boards and pages |
| `<Text>` | `text` | none |
| `<Image>` | `image` | none |
| `<Vector>` | `vector` | none |
| `<Instance>` | `instance` | none |

Attributes are the node's properties, written `name="text"` or
`name={expression}`. A tag that takes an attribute that it does not list is
rejected, and so is an attribute written twice. An attribute written without a
value, such as `clip`, is `true`.

Every tag takes `id`, `name` and `visible`. Every tag except `<Page>` takes the
placement and size attributes, and `<Board>` and `<Frame>` take every
attribute of the layout and style tables.

### Identity

| Attribute | Value |
| --- | --- |
| `id` | The node's ID, 8 lowercase base32 characters, such as `"k3f9x2qa"`. It refers to an existing node. A tag written without `id` is a new node: the server assigns its ID (US-031), never the agent. Each ID is written once. |
| `name` | The node's name, as text. Required. |
| `visible` | `{false}` hides the node. Left out, the node is visible. |

### Placement

| Attribute | Value |
| --- | --- |
| `absolute` | Takes the node out of the flow of its flex or grid parent. |
| `x`, `y` | The node's position in its parent, as numbers of pixels. Only the boards of a page, the children of a layout without a mode and children with `absolute` take them: layout computes the position of every child that flows, so `x` and `y` on a child that flows are an error. |
| `gridColumn`, `gridRow` | The grid lines of a child of a grid, an object with `start` (a line from 1, or from -1 at the end) and `span`: `gridColumn={{start: 1, span: 2}}`. |
| `justifySelf`, `alignSelf` | How a child of a grid aligns itself in its grid area: `start`, `center`, `end` or `stretch`. |
| `margin` | The space outside the node: one length for every side, `margin={8}`, or an object with some of `top`, `right`, `bottom` and `left`, which may be negative. |

### Size

| Attribute | Value |
| --- | --- |
| `width`, `height` | `"hug"` (the default), `"fill"`, a number of pixels, `width={320}`, or a token reference, `width="$size.card"`. An instance that leaves them out takes its component's size. |
| `minWidth`, `maxWidth`, `minHeight`, `maxHeight` | A length of 0 or more, or a token reference. |

### Layout (`<Board>` and `<Frame>`)

A board or frame without any layout attribute has no layout: its children sit
at their own `x` and `y`.

| Attribute | Value |
| --- | --- |
| `layout` | `"none"`, `"flex"` or `"grid"`. Left out, the mode is `grid` when `cols` or `rows` is set, `flex` when `dir`, `wrap`, `align` or `justify` is set, and `none` otherwise. Say it when no other attribute does: `layout="flex"` with only `gap`. |
| `dir` | The direction of a flex layout: `"row"`, `"col"`, `"row-reverse"` or `"col-reverse"`. |
| `wrap` | `"nowrap"`, `"wrap"` or `"wrap-reverse"`. |
| `gap` | The space between rows and between columns: one length for both, `gap="$space.md"` or `gap={8}`, or an object with `row` and `column`, `gap={{row: 8, column: 16}}`. |
| `pad` | The padding: one length for every side, `pad={16}`, or an object with some of `top`, `right`, `bottom` and `left`, `pad={{top: 8, left: 16}}`. |
| `align` | How a flex layout aligns children on the cross axis: `"start"`, `"center"`, `"end"`, `"stretch"` or `"baseline"`. |
| `justify` | How a flex layout spreads children along the main axis: `"start"`, `"center"`, `"end"`, `"space-between"`, `"space-around"` or `"space-evenly"`. |
| `cols`, `rows` | The tracks of a grid, a list of `{fixed: 240}`, `{fr: 1}` and `"auto"`: `cols={[{fixed: 240}, {fr: 1}]}`. |

`dir`, `wrap`, `align` and `justify` are for flex layouts, and `cols` and
`rows` for grids. `gap` and `pad` fit both. `docs/formats/layout.md` maps each
to CSS.

### Style

| Attribute | Value |
| --- | --- |
| `fill` | One solid color, `fill="#2563eb"` or `fill="$color.surface"`, or a list of fills, painted in order: `fill={[{type: "solid", color: "#2563eb"}, {type: "linear-gradient", angle: 180, stops: [{position: 0, color: "#fff"}, {position: 1, color: "#000"}]}]}`. A fill can also be `{type: "image", image: "attachment:<id>", fit: "cover"}`. Boards, frames and vectors take it. |
| `strokes` | A list of strokes: `strokes={[{color: "#e2e8f0", width: 1, align: "inside", dash: {length: 4, gap: 2}}]}`. `align` (`inside`, `center` or `outside`) is for the strokes of boards and frames. |
| `radius` | The corner radius: one length for every corner, `radius={8}`, or an object with some of `topLeft`, `topRight`, `bottomRight` and `bottomLeft`. |
| `shadows` | A list of shadows or token references: `shadows={[{type: "drop", color: "#0f172a33", offsetX: 0, offsetY: 4, blur: 8, spread: 0}, "$shadow.raised"]}`. |
| `opacity` | From 0 to 1, or a token reference. |
| `clip` | Clips the children to the box. |

### Text (`<Text>`)

`content`, `fontFamily` and `fontSize` are required.

| Attribute | Value |
| --- | --- |
| `content` | The text, exactly as written, never markup. |
| `fontFamily` | The family's name, `"Inter"`, or a token reference. |
| `fontSize` | A length of 0 or more, or a token reference. |
| `fontWeight` | From 1 to 1000, or a token reference. |
| `fontStyle` | `"normal"` or `"italic"`. |
| `lineHeight` | A multiple of the font size, or a token reference. |
| `letterSpacing` | A length, which may be negative, or a token reference. |
| `textAlign` | `"left"`, `"center"`, `"right"` or `"justify"`. |
| `textDecoration` | `"none"`, `"underline"` or `"line-through"`. |
| `color` | A color or a token reference. |

### Images, vectors and instances

| Attribute | Value |
| --- | --- |
| `image` | The attachment an `<Image>` shows, `"attachment:<id>"`. Required. |
| `fit` | `"cover"` (the default) or `"contain"`. |
| `path` | The SVG path data of a `<Vector>`, never SVG markup. Required. |
| `component` | The ID of the component an `<Instance>` places. Required. |

## Values

- **Text** is written between double quotes. A backslash writes what cannot
  stand for itself: `\"` for a quote, `\\` for a backslash, `\n` for a new
  line, `\r`, `\t`, and `\u{1f6d2}` for the character with that hexadecimal
  code point. Everything else, such as `{`, `<`, `&` and emoji, stands for
  itself.
- **Expressions** are written in braces and hold a number (`16`, `-0.5`,
  `1e-7`), text, `true`, `false`, a list (`[1, 2]`) or an object
  (`{row: 8, column: 16}`). Object keys need no quotes. A number is read as the
  exact double, and printed as the shortest text that reads back as the same
  double. Text in quotes is the same as text in an expression, so `dir="col"`
  is `dir={"col"}`.
- **Colors** are `#rrggbb` or `#rrggbbaa`.
- **Token references** are `$` and the token's path, such as `$color.surface`.
  Every color, every length except `x` and `y`, every opacity and font value
  and a whole shadow may be a token reference.
- Literals keep the ranges of the JSON form: sizes, gaps, padding, radii and
  font sizes are 0 or more, opacity goes from 0 to 1, and margins, `x`, `y` and
  letter spacing may be negative.

## Printing

`tags::print` writes one tag per node, one per line, indented two spaces per
level, with the `id` of every node. It writes every property a node sets, and
leaves out what leaving out means: `visible` when it is true, `width` and
`height` when they are `"hug"` (except on instances), `fit` when it is
`"cover"`, and the layout mode when another attribute says it. One value for
every side or corner is written as the short form (`pad={16}`), a single solid
fill as `fill="#2563eb"`, and `true` as the bare attribute.

## Errors

A tag that does not parse fails with an error that names the line, the column
and how to fix it. Lines and columns count from 1, and columns count
characters, not bytes. The error points at the first character of the tag
name, of the attribute name, or of the value (its opening `"` or `{`) that has
the problem:

```
line 2, column 3: `Box` is not a tag
fix: use one of `Page`, `Board`, `Frame`, `Text`, `Image`, `Vector` or `Instance`
```

Tags are rejected for an unknown tag or attribute, an invalid value, an `id`
written on two nodes, a tag left open or closed with the wrong tag, a board
anywhere but on a page, `x` or `y` on a child that flows in a flex or grid
layout, grid placement on a child of a node that is not a grid, a missing
required attribute, and for text left over between tags.
