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
| `<Component>` | `component` | any node except boards, pages and components |

Attributes are the node's properties, written `name="text"` or
`name={expression}`. A tag that takes an attribute that it does not list is
rejected, and so is an attribute written twice. An attribute written without a
value, such as `clip`, is `true`.

Every tag takes `id`, `name` and `visible`. Every tag except `<Page>` and
`<Component>` takes the placement and size attributes, and `<Board>`, `<Frame>`
and `<Component>` take every attribute of the layout and style tables. A
`<Component>` takes the size, layout and style attributes of a frame, and
`props`; it is placed by its instances, so it has no placement attributes.

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
| `strokes` | A list of strokes: `strokes={[{color: "#e2e8f0", width: 1, align: "inside", dash: {length: 4, gap: 2}}]}`. `align` (`inside`, `center` or `outside`) is for the strokes of boards, frames and vectors with closed paths; left out it is `inside`. The stroke of an open path is centered whatever `align` says. `cap` (`butt`, `round` or `square`; left out it is `butt`) is for vectors: it shapes the ends of the stroke and every dash, so `dash: {length: 0, gap: 8}` with `cap: "round"` draws dots. A board or frame accepts `cap` and ignores it. |
| `radius` | The corner radius: one length for every corner, `radius={8}`, or an object with some of `topLeft`, `topRight`, `bottomRight` and `bottomLeft`. |
| `shadows` | A list of shadows or token references: `shadows={[{type: "drop", color: "#0f172a33", offsetX: 0, offsetY: 4, blur: 8, spread: 0}, "$shadow.raised"]}`. |
| `opacity` | From 0 to 1, or a token reference. |
| `clip` | Clips the children to the box. |
| `overflow` | How a board or frame scrolls its content: `"none"`, `"horizontal"`, `"vertical"` or `"both"`; left out it is `none`. Boards and frames take it. |
| `modes` | The mode of each collection that a board is resolved in, such as `modes={{color: "dark"}}`, whatever mode the page is shown in. Only boards take it. A write refuses a collection the design system does not have and a mode the collection does not list. |
| `scrollPosition` | Whether a layer inside a board scrolls with it, `"scrolls"`, stays `"fixed"` or is `"sticky"`; left out it is `scrolls`. Pages, boards and components refuse it, and so do the layers of a component. |

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
| `shadows` | A list of shadows, as for boards. Each drop shadow is cast from the glyphs, as CSS `text-shadow`: its `spread` is taken as 0, and an inner shadow draws nothing. |

### Images, vectors and instances

| Attribute | Value |
| --- | --- |
| `image` | The attachment an `<Image>` shows, `"attachment:<id>"`. Required. |
| `fit` | `"cover"` (the default) or `"contain"`. |
| `path` | The SVG path data of a `<Vector>`, never SVG markup. Required. |
| `component` | The ID of the component an `<Instance>` places. Required. |

## Components and instances

A component is a tree of layers with properties, defined by a `<Component>` tag
at the top of the design system doc (and nowhere else: a `<Component>` inside a
page, a board, a frame or another component is an error at its tag name). An
`<Instance>` in a design places a component. The instance stores only the
component it places, the values of its properties and its overrides, never a
copy of the component's tree or style.

### Properties (`props` of a `<Component>`)

`props` is an object that maps a property's name to its definition, with a
`type` of one of four kinds, and a `default`:

| Kind | Definition | An instance's value |
| --- | --- | --- |
| `variant` | `{type: "variant", values: ["primary", "secondary"], default: "primary"}`: one of a fixed set of values, and the default is one of them. | One of the values, as text. |
| `boolean` | `{type: "boolean", default: true}`: shows or hides a layer. | `true` or `false`. |
| `text` | `{type: "text", default: "Buy"}`: replaces a label. | Text. |
| `slot` | `{type: "slot", default: "k3f9x2qa"}`: swaps in another component, whose node ID is the default. | The node ID of a component. |

A type other than these four is an error at the `props` value, and its fix
lists the four kinds. So is a `variant` default that is not one of its
`values`, and a `slot` default that is not a node ID. In JSON a property is
written the same way, and `props` is the `props` of the `component` node.

Components with the same name and the same variant properties (the same names
with the same `values`) are the variants of one set, and each one's defaults are
its own variant values. An instance shows the variant of its component's set
whose defaults equal the instance's values for the variant properties, with the
component's own defaults where the instance leaves one out. A component with
another name is never in the set, whatever properties it has, and the component
the instance names is shown when it matches, and when nothing does. The
renderer follows this (`crates/omni-render/README.md`, "Instances and tokens"),
and so should the Penpot import (US-051) when it maps variant sets.

### Layers (`bind`)

A layer of a component takes a property with `bind`, an object with some of
these fields, each naming a property of the component:

| Field | Layer | Property | Effect |
| --- | --- | --- | --- |
| `visible` | any layer | a `boolean` | shows or hides the layer |
| `content` | a `<Text>` | a `text` | replaces the text of the label |
| `component` | an `<Instance>` | a `slot` | swaps the component the instance places |

`bind` is written on the `<Frame>`, `<Text>`, `<Image>`, `<Vector>` and
`<Instance>` layers of a component. A `bind` to a property the component does
not define, to a property of the wrong kind, or of `content` or `component` on
a layer of another type is an error at the `bind` value that names the
property or the field, and `bind` outside a component is an error too. A
component that sets only `gap` or `pad` and no other layout attribute lays out
its layers as a flex layout.

```
<Component id="btn00001" name="Button" dir="row" gap={8} pad={12} fill="#2563eb" radius={8} props={{style: {type: "variant", values: ["primary", "secondary"], default: "primary"}, showIcon: {type: "boolean", default: true}, label: {type: "text", default: "Buy"}, icon: {type: "slot", default: "cart0001"}}}>
  <Instance id="gfx00001" name="Icon" component="cart0001" bind={{visible: "showIcon", component: "icon"}}/>
  <Text id="txt00001" name="Label" content="Buy" fontFamily="Inter" fontSize={14} color="#ffffff" bind={{content: "label"}}/>
  <Frame id="bdg00001" name="Badge" width={8} height={8} fill="#dc2626"/>
</Component>
```

### Interactions (`interactions`)

`interactions` is a list of what a layer does when the person using a
prototype touches it. Every tag except `<Page>` takes it: `<Board>`, `<Frame>`,
`<Text>`, `<Image>`, `<Vector>`, `<Instance>` and `<Component>`, and the layers
inside a component. A `<Page>` rejects it. The names are FigCraft's
(`crates/doc/src/proto.rs`, MIT OR Apache-2.0; see `NOTICE`).

```
<Frame id="btn00001" name="Buy" width={120} height={40} interactions={[{trigger: {type: "onClick"}, actions: [{type: "navigate", dest: "k3f9x2qa", preserveScroll: true}], transition: {type: "slideIn", direction: "left"}, durationMs: 300, easing: "easeOut"}]}/>
```

An interaction has `actions` (required) and these fields, which printing
leaves out when they hold the default:

| Field | Values | Default |
| --- | --- | --- |
| `trigger` | an object with a `type`: `onClick`, `onHover`, `onPress`, `onDrag`, `mouseEnter`, `mouseLeave`, `mouseDown`, `mouseUp`, `afterDelay` (`ms`) or `keyPress` (`key`, such as `"Shift+K"`); a bare name such as `"onClick"` is shorthand for a trigger without fields | `onClick` |
| `transition` | an object with a `type`: `instant`, `dissolve`, `smartAnimate`, or `moveIn`, `moveOut`, `push`, `slideIn`, `slideOut` with a `direction` of `left`, `right`, `top` or `bottom` | `instant` |
| `durationMs` | a whole number of milliseconds | `300` |
| `easing` | `linear`, `easeIn`, `easeOut`, `easeInOut`, `easeInBack`, `easeOutBack`, `easeInOutBack`, `gentle`, `quick`, `bouncy`, `slow`, or an object `{type: "customBezier", x1, y1, x2, y2}` or `{type: "customSpring", mass, stiffness, damping}` | `easeOut` |
| `delayMs` | a whole number of milliseconds | `0` |

An action has a `type`, and each type takes these fields; a field with a
default is left out when it holds it:

| Action | Fields |
| --- | --- |
| `navigate` | `dest`, `preserveScroll` (`false`) |
| `back` | none |
| `scrollTo` | `dest`, `offset` (`[0, 0]`) |
| `openOverlay` | `dest`, `position` (`center`, `topLeft`, `topCenter`, `topRight`, `bottomLeft`, `bottomCenter`, `bottomRight`, `manual`; default `center`), `closeOnOutside` (`true`), `background` (a color), `offset` (a pair, `[12, 40]`), `anchor` (`screen`, `triggerInside`, `triggerOutside`; default `screen`) |
| `swapOverlay` | `dest` |
| `closeOverlay` | none |
| `openUrl` | `url`, `newTab` (`true`) |
| `changeTo` | `dest` |
| `setVariable` | `variable`, and `value` or `expr` |
| `setVariableMode` | `collection`, `mode` |
| `conditional` | `blocks`, a list of `{condition, actions}`; a block without `condition` always runs |

A `dest` is the ID of a node and is not checked against the design yet. The
variables, collections, modes and expressions of the three variable actions
are checked when a layer is written, against the design system's tokens (a
design system without tokens has no variables), with the same messages for the
actions in the blocks of a `conditional`, at any depth:

- `setVariable` takes the path of a token (`state.count`) of type number,
  string, boolean or color, and either a `value` of that type (a color token
  takes a color written as text) or an `expr` that yields that type, not both.
- `setVariableMode` takes a `collection` (a top-level group) that has modes,
  and one of its `mode`s.
- `conditional` takes at least one block; only the last block may leave out its
  `condition`, which makes it the else branch. A `condition` yields a boolean.

An expression is a JSON tree, nested at most 16 levels (the root is level 1):

| Expression | Yields |
| --- | --- |
| `{value: 1}`, `{value: "a"}`, `{value: true}` | a number, a string, a boolean |
| `{variable: "state.count"}` | the type of the token |
| `{op, args}` | see below |
| `{mode: "color"}` | a string: the collection's current mode |
| `{inMode: {variable: "color.surface", mode: "dark"}}` | the type of the token, in that mode |

The operators take one argument (`not`, `negate`) or two (the rest):
`add`, `subtract`, `multiply` and `divide` take numbers and yield a number;
`less`, `lessOrEqual`, `greater` and `greaterOrEqual` take numbers and yield a
boolean; `and` and `or` take booleans; `not` takes a boolean; `negate` takes a
number; `equals` and `notEqual` compare two values of one type. An unknown
trigger, action, transition or easing, and a missing required field, are
errors that list the names to write instead, such as "`tap` is not a trigger:
write one of onClick, onHover, ...".

### Flows and prototype settings (`flows` and `prototype` of a `<Page>`)

A `<Page>` takes `flows`, a list of the flows of its prototype, and `prototype`,
how the page is shown in play mode. Play mode offers the flows as entry points;
a page without flows plays from the board that is open. A flow has a `name`, a
`start` (the ID of one of the page's boards) and a `description`, which printing
leaves out when it is empty. Two flows may not share a name, and a write of
`flows` is refused when a `start` is not a board of the page. A board deleted
afterwards leaves its flow as it was: reading prints it unchanged.

```
<Page id="pg000001" name="Shop" flows={[{name: "Checkout", start: "b1a2c3d4", description: "From the cart to the receipt"}]} prototype={{device: "iPhone 16", color: "Black", rotation: "landscape", background: "#000000", responsive: true}}>
```

`prototype` is an object with some of these fields; printing leaves out a field
that holds its default, and a `prototype` of defaults only, and an empty `flows`,
print nothing:

| Field | Values | Default |
| --- | --- | --- |
| `device` | a name of the device table below | `None` |
| `color` | a colour of the device | the device's first |
| `rotation` | `portrait`, or `landscape` for a device that rotates | `portrait` |
| `background` | a color | `#1e1e1e` |
| `responsive` | `true` or `false` | `false` |
| `customSize` | `[width, height]` in points, only for the device `Custom`, which needs it | none |

The device table is FigCraft's (`crates/doc/src/proto.rs`, MIT OR Apache-2.0;
see `NOTICE`), in `crates/omni-core/src/design/devices.rs`: iPhone 17, 17 Pro,
17 Pro Max, iPhone Air, iPhone 16, 16 Pro, 16 Pro Max, 16 Plus, iPhone 15 Pro,
15 Pro Max, 15 Plus, iPhone 14 Pro, 14 Pro Max, 14, Android Compact, Android
Medium, iPad mini, MacBook Air, Desktop, and the two choices that are not
models: `Custom` (the screen is `customSize`) and `None` (a board at its own
size). Phones and tablets rotate; MacBook Air, Desktop, `Custom` and `None` do
not. A device outside the table, a `customSize` on another device, a rotation
the device does not take and a colour it does not come in are errors that name
what to write instead.

The doc outline (`GET .../docs/{doc}/html?view=outline`) carries `flows` and
`prototype` on each page, so the viewer reads them without another request. A
page written without them has `flows: []` and `prototype` with every field:
`device`, `color` (`null`), `rotation`, `background`, `responsive` and
`customSize` (`null`), the defaults filled in.

### Instances (`props` and `overrides` of an `<Instance>`)

An `<Instance>` takes `component` (the component's node ID), `props` (a value
for each property it sets, as in the table above) and `overrides`. It takes no
style of its own and has no children. `overrides` is an object that maps the ID
of a node of the component to what the instance changes on it, with these
fields only:

| Field | Value | On |
| --- | --- | --- |
| `content` | The text. | a text node |
| `fills` | A list of fills, as the `fill` of a `<Frame>`. | a node with fills |
| `visible` | `true` or `false`. | any node |
| `component` | The node ID of a component: the contents of a slot. | an instance node |

An override key that is not a node ID, any other field, an invalid fill, and a
property value that is a list or an object are errors at the value that names
them.

```
<Instance id="pay00001" name="Pay button" component="btn00001" props={{style: "secondary", showIcon: false, label: "Pay", icon: "hrt00001"}} overrides={{txt00001: {content: "Pay now"}, bdg00001: {fills: [{type: "solid", color: "#16a34a"}]}, gfx00001: {visible: false, component: "hrt00001"}}}/>
```

The API checks every instance against the component it resolves to, and
refuses an unknown component, property, variant value or slot target, an
override of a node that is not in the component, and a `content` override of a
node that is not text. A design resolves its instances against the design
system's head as it is when the design is read.

### Addresses

A node inside an instance is addressed as `<instanceId>/<componentNodeId>`:
the ID of the instance, a `/`, and the ID of the component's node. Overrides
are keyed by the component's node IDs, so each override key, joined to its
instance's ID, is the address of the node it changes. Comments and the
renderer address nodes this way too: the renderer names a node inside an
instance inside an instance by the instance on the page.
`omni_core::design::NodeAddress` parses `<id>` and
`<instanceId>/<componentNodeId>` and rejects anything deeper or malformed.

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

When an agent reads a board of a design (`omni design read`), every tag also
carries `rev={n}` after its `id`: the node's revision, which an edit lists as
the revision it expects. A node starts at revision 1, and each later import
that changes what the node itself holds (not its children) adds one, as does
each `omni design edit` operation that changes, replaces or moves the node
(docs/formats/operations.md, Designs); adding or moving a child leaves its
parent's revision alone. `rev` is only read, never written: a tag that sets it
is refused.

## Boards as chunks

The head of a design is a manifest of chunks (US-030). Each page and each board
is stored as a chunk of its own in RustFS, named by the BLAKE3 hash of its
bytes: a board's chunk holds the board and its subtree, a page's chunk the page
without its boards, and the head's own chunk lists them all, in order. A page or
a board that a later write does not change keeps its chunk, so heads share them
and a new head stores only what changed. `omni design read` prints
one board in the tag format, from its one chunk; a board is addressed by its
node ID (`<instanceId>/<componentNodeId>` addresses reach into instances).

## Errors

A tag that does not parse fails with an error that names the line, the column
and how to fix it. Lines and columns count from 1, and columns count
characters, not bytes. The error points at the first character of the tag
name, of the attribute name, or of the value (its opening `"` or `{`) that has
the problem:

```
line 2, column 3: `Box` is not a tag
fix: use one of `Page`, `Board`, `Frame`, `Text`, `Image`, `Vector`, `Instance` or `Component`
```

Tags are rejected for an unknown tag or attribute, an invalid value, an `id`
written on two nodes, a tag left open or closed with the wrong tag, a board
anywhere but on a page, `x` or `y` on a child that flows in a flex or grid
layout, grid placement on a child of a node that is not a grid, a missing
required attribute, and for text left over between tags.
