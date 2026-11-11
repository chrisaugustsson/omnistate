# Designs

A `design` doc holds pages of boards (screens, states) as nodes in a JSX-like tag format. A `design-system` doc (one per project, app level, `ui` pillar) holds the design tokens and components that designs refer to. Designs are read and written as tags; the web app renders them.

## Reading

- `omni doc read <doc>` prints the whole head as tags. `omni design read --doc <doc> --board <board>` prints one board; use it on large designs.
- `omni doc read <doc> --node <id>` (repeatable) prints only the tags with those IDs, each with its children and without `rev`; read the board before you edit.
- A new design doc starts with one `<Page id="..." name="Page 1"/>`; boards go under it.

## Tags

```
<Board name="Cart" x={0} y={0} width={390} height={640} dir="col" gap={12} pad={16} fill="#ffffff">
  <Text name="Title" content="Your cart" fontFamily="Inter" fontSize={24} fontWeight={600}/>
  <Frame name="Row" dir="row" gap={8} align="center">
    <Text name="Item" content="Winter tyre" fontFamily="Inter" fontSize={16}/>
    <Instance name="Remove" component="rtcg443s"/>
  </Frame>
</Board>
```

- Tags: `Page` (holds boards), `Board` (a screen; on a page only), `Frame` (a group or container), `Text`, `Image`, `Vector`, `Instance` (a component placed), `Component` (design system only). Every tag needs a `name`. `Text` needs `content`, `fontFamily` and `fontSize`.
- Layout is flex or grid: `dir` (`row`|`col`), `gap`, `pad`, `align`, `justify`, `wrap`; `cols`/`rows` for a grid. Size is `width`/`height`: a number, `"hug"` (default) or `"fill"`. Children that flow take no `x`/`y`; boards on a page do, so give each board its own `x` and `y` (spaced apart, such as 0, 450, 900) or they overlap.
- Style: `fill="#rrggbb"` (colors are `#rrggbb` or `#rrggbbaa`, six or eight digits), `strokes`, `radius`, `shadows`, `opacity`, `clip`. A value in braces is a number, list or object: `pad={16}`, `gap={{row: 8, column: 16}}`.
- Tokens: `$color.brand` as a value (`fill="$color.brand"`, `gap="$space.md"`) refers to the design system's token tree. `omni tokens export` shows it; `omni tokens import <file.json>` replaces the whole tree (DTCG JSON; modes in `$extensions.omnistate.modes`); `omni tokens set <path> <value> [--mode <m>] [--type <t>]` and `omni tokens remove <path>` change one token (`--type` creates a missing one).
- Components: define them in the design system with `PUT /api/v1/w/<workspace_id>/projects/<project_id>/components`, body `{"tags": "<Component ...>...</Component>"}`, which replaces all components (send the existing ones with their `id` to keep them): `jq -Rs '{tags:.}' components.tags | omni api PUT <path> --body-file -`. Place one with `<Instance name=".." component="<component id>" props={{label: "Pay"}} overrides={{<component node id>: {content: "Pay now"}}}/>`. `omni component usage <component>` lists the boards that use it.
- New tags carry no `id`; the server assigns IDs and prints them.

## Prototyping

A layer reacts to the person using the prototype through `interactions`; a page names where its prototypes start through `flows`. Both are attributes, written with the `set`, `replace` and `insert` operations below and printed by `omni design read`. Names are camelCase. A `trigger` is an object with a `type` (`onClick`, `onHover`, `onPress`, `afterDelay`, `keyPress`, ...); a bare `"onClick"` is accepted, `click` is not. Defaults: trigger `onClick`, transition `instant`, `durationMs` 300, `easing` `easeOut`. Each `dest` is the ID of a node and is checked on write: a board for `navigate`, `openOverlay` and `swapOverlay`. The full lists of triggers, actions, transitions and easings are in `docs/formats/design-tags.md`, `### Interactions`.

A button that navigates to another board with a slide:

```tags
<Frame name="Pay" width={120} height={40} fill="#2563eb" radius={8} interactions={[{trigger: {type: "onClick"}, actions: [{type: "navigate", dest: "cnf00002"}], transition: {type: "slideIn", direction: "left"}, durationMs: 300, easing: "easeOut"}]}/>
```

A layer that opens an overlay, centred, with a dimmed background (a `closeOverlay` action goes on a layer inside the overlay board):

```tags
<Frame name="Promo link" interactions={[{trigger: {type: "onClick"}, actions: [{type: "openOverlay", dest: "prm00003", position: "center", closeOnOutside: true, background: "#00000080"}]}]}/>
```

A page lists its flows and how play mode shows it. A flow has a `name`, a `start` (the ID of a board of the page) and an optional `description`; names are unique. `prototype` takes `device`, `color`, `rotation`, `background`, `responsive` and `customSize`:

```tags
<Page name="Shop" flows={[{name: "Checkout", start: "crt00001", description: "From the cart to the receipt"}]} prototype={{device: "iPhone 16", rotation: "portrait", background: "#1e1e1e"}}/>
```

A board or frame scrolls with `overflow="vertical"` (`none`, `horizontal`, `vertical`, `both`); a layer inside it stays put with `scrollPosition="fixed"` (`scrolls`, `fixed`, `sticky`).

Read the graph back with `omni design flows --doc <doc> [--page <page>]`: the flows, each board's interactions with their targets, boards that no flow start or interaction reaches, and dangling targets (a deleted board, marked `(missing)`):

```sh
omni design flows --doc <doc>
#   flows: Checkout → Cart (crt00001)
#     Pay button (pay00001) onClick → navigate Confirmation (cnf00002) · slideIn left · 300 ms · easeOut
#   unreachable: Settings (stg00004)
```

Done when `omni design flows` shows no dangling target and no unreachable board except those that start a flow.

## Variables

The design system's token tree is the variable model. Each top-level group is a collection with its own modes (`$extensions.omnistate.modes` on the group, `[]` for none). The types `number`, `string` and `boolean` hold prototype state; a token with a value per mode lists them in `$extensions.omnistate.modes` on the token, one for each mode of its collection. The root lists at least one mode, which a group without modes of its own uses. This tree has a `state` collection and a `density` collection with two modes (`omni tokens import` replaces the whole tree; the format is `docs/formats/tokens.md`, `## Collections and modes`):

```json
{
  "$extensions": {"omnistate": {"modes": ["default"]}},
  "state": {
    "$extensions": {"omnistate": {"modes": []}},
    "isOpen": {"$type": "boolean", "$value": false},
    "count": {"$type": "number", "$value": 0},
    "title": {"$type": "string", "$value": "Your cart"}
  },
  "density": {
    "$extensions": {"omnistate": {"modes": ["comfortable", "compact"]}},
    "space": {
      "md": {
        "$type": "dimension",
        "$value": "16px",
        "$extensions": {"omnistate": {"modes": {"comfortable": "16px", "compact": "12px"}}}
      }
    }
  }
}
```

Bind a field to a token with `$path`: `content`, `visible`, `opacity`, a stroke's width, the text measures and `rotation` take it next to the colors and lengths. The token must exist and have the field's type, or the write is refused. `bind` stays for a component's own props.

```tags
<Frame name="Panel" visible="$state.isOpen" dir="col" gap="$density.space.md">
  <Text name="Title" content="$state.title" fontFamily="Inter" fontSize={24}/>
</Frame>
```

`setVariable` changes a token for the play session only, with a `value` or an `expr` (a JSON tree: `{value}`, `{variable}`, `{op, args}` with `add`, `subtract`, `equals`, `less`, `and`, `not`, ...; also `{mode}` and `{inMode}`). `setVariableMode` switches a collection's mode (`collection`, `mode`):

```tags
<Frame name="Add" width={120} height={40} interactions={[{trigger: {type: "onClick"}, actions: [{type: "setVariable", variable: "state.count", expr: {op: "add", args: [{variable: "state.count"}, {value: 1}]}}, {type: "setVariable", variable: "state.isOpen", value: true}]}]}/>
```

A `conditional` action takes `blocks` of `{condition, actions}`; only the last block may leave out `condition`, as the else branch. A board takes `modes={{density: "compact"}}` to resolve its subtree in that mode whatever mode the page shows.

Change one token without resending the tree; `--type <t>` creates a token that does not exist yet:

```sh
omni tokens set state.count 3
omni tokens set density.space.md 12px --mode compact
omni tokens remove state.title
```

## Writing

`omni design edit <doc> --body-file <file>` (or `omni doc edit`) takes a JSON array of operations, all or nothing, like an html doc (see [html-docs.md](html-docs.md)):

- `insert` a subtree: `{"op":"insert","parent":"<page or board or frame id>","position":{"after":null},"content":"<Board .../>"}`. A board goes under a page, anything else under a board or frame.
- `set` changes attributes by name, `null` removes one: `{"op":"set","node":"<id>","rev":1,"properties":{"content":"Hej","fontSize":18}}`.
- `replace` swaps a subtree (tags that keep an `id` stay; tags without one are new), `move` and `delete` as for html.
- The results print the new revision of each node and the IDs of new nodes. A node you just inserted is at revision 1.

Build a board in a file and insert it with `jq`:

```
jq -Rs --arg page <page id> '[{op:"insert",parent:$page,position:{after:null},content:.}]' board.tags | omni design edit <doc> --body-file - --as <run>/<role>
```

Revisions: when a read shows `rev={n}` on a tag, send that number. When it shows none, send `1` for a node you have not edited; a stale one is refused and the error names the current revision, so resend with that.

`omni import penpot <file.penpot> --project <id> [--feature <name>]` brings a Penpot file in as a design doc with its tokens and components.
