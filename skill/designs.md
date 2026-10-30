# Designs

A `design` doc holds pages of boards (screens, states) as nodes in a JSX-like tag format. A `design-system` doc (one per project, app level, `ui` pillar) holds the design tokens and components that designs refer to. Designs are read and written as tags; the web app renders them.

## Reading

- `omni doc read <doc>` prints the whole head as tags. `omni design read --doc <doc> --board <board>` prints one board; use it on large designs.
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
- Tokens: `$color.brand` as a value (`fill="$color.brand"`, `gap="$space.md"`) refers to the design system's token tree. `omni tokens export` shows it; `omni tokens import <file.json>` replaces the whole tree (DTCG JSON; modes in `$extensions.omnistate.modes`).
- Components: define them in the design system with `PUT /api/v1/w/<workspace_id>/projects/<project_id>/components`, body `{"tags": "<Component ...>...</Component>"}`, which replaces all components (send the existing ones with their `id` to keep them): `jq -Rs '{tags:.}' components.tags | omni api PUT <path> --body-file -`. Place one with `<Instance name=".." component="<component id>" props={{label: "Pay"}} overrides={{<component node id>: {content: "Pay now"}}}/>`. `omni component usage <component>` lists the boards that use it.
- New tags carry no `id`; the server assigns IDs and prints them.

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
