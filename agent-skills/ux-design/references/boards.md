# Boards: drawing in an OmniState design doc

A design doc holds pages; a page holds boards (one screen or state each); a board holds layers. You write them as tags with the `omni` CLI and see the result as an image. `ux-design` (grey options) and `ui-design` (the finished design) both draw this way.

`omni --help` lists every command and flag. Every command takes the caller's identity right after the word `omni`: `omni --as <run>/<role> ...`. A refused write changes nothing and its error names what to write instead: fix that operation and send the batch again.

## Tags

```
<Board name="Rounds" x={0} y={0} width={390} height={844} dir="col" gap={12} pad={16} fill="#ffffff" overflow="vertical">
  <Text name="Title" content="Your rounds" fontFamily="Inter" fontSize={24} fontWeight={600} color="#111111"/>
  <Frame name="Row" width="fill" dir="row" gap={8} align="center" justify="space-between">
    <Text name="Course" content="Järva DGP, 18 holes" fontFamily="Inter" fontSize={16} color="#111111"/>
    <Instance name="Open" component="rtcg443s" props={{label: "Open"}}/>
  </Frame>
</Board>
```

- **Tags:** `Page`, `Board` (under a page only), `Frame` (a container), `Text`, `Image` (`image="attachment:<id>"`, from `omni attach <file>`), `Vector` (`path`, SVG path data), `Instance` (a component placed). Every tag has a `name`; a `Text` also has `content`, `fontFamily` and `fontSize`. New tags carry no `id`: the server assigns IDs and prints them.
- **Layout:** `dir` (`row` or `col`), `gap`, `pad`, `align`, `justify`, `wrap`; `cols` and `rows` for a grid. A frame with `gap` or `pad` states its `dir`. Children of a layout flow and take no `x` or `y`.
- **Size:** `width` and `height` are a number, `"hug"` (the default) or `"fill"`.
- **A board** has its own `x`, `y`, `width` and `height` on the page. Content taller than the screen: keep the board at the screen's height and set `overflow="vertical"`; a bar that stays in view takes `scrollPosition="fixed"`.
- **Style:** `fill`, `strokes={[{color: "#cccccc", width: 1}]}`, `radius`, `shadows`, `opacity`, `clip`. Colours are `#rrggbb` or `#rrggbbaa`.
- **Values:** a number, list or object goes in braces (`pad={16}`, `pad={{top: 8, left: 16}}`), text in double quotes. Write text without backslashes: a backslash escapes the next character, the closing quote included.
- **Tokens:** `$` and a token's path as the value (`fill="$color.surface"`, `gap="$spacing.md"`) refers to the project's design system. `omni tokens export` prints the tree. A project without a design system refuses every reference.
- **Instances:** `<Instance name="Pay" component="<component id>" props={{label: "Pay"}}/>` places a component of the design system. It has no children and no style of its own; `props` sets the component's properties and `overrides={{<component node id>: {content: "Pay now"}}}` changes one layer's text, fills or visibility.

## Writing

Put the tags of one board in a file and send it as one operation:

```sh
jq -Rs --arg page <page id> '[{op:"insert",parent:$page,position:{after:null},content:.}]' board.tags \
  | omni --as <run>/<role> design edit <doc> --body-file -
```

- A new design doc holds one empty page; `omni doc read <doc>` prints its ID. A further page is an insert at the top level: `{"op":"insert","parent":null,"position":{"after":"<last page id>"},"content":"<Page name=\"Round 2\"/>"}`.
- `position.after: null` puts the new node first, so several boards sent that way end up in reverse order in the doc's outline. Their place on the canvas is their `x` and `y`.
- The result prints the ID of every new node, the board's first. Keep the board IDs: interactions, flows and reports name boards by ID.
- Work files (the `.tags` files, images) go in the scratch folder the caller names, otherwise in a folder from `mktemp -d`.

## Changing a board that exists

Read it first: `omni --as <run>/<role> design read --doc <doc> --board <board>` prints its tags with the `id` of every layer. Then send only what changes, so the IDs stay and the human's comments stay on their layers:

- `{"op":"set","node":"<id>","rev":<rev>,"properties":{"content":"Save round","fill":"$color.primary"}}` changes attributes; `null` removes one.
- `{"op":"replace","node":"<id>","rev":<rev>,"content":"<Frame ...>...</Frame>"}` swaps a subtree; tags that keep their `id` stay the same nodes.
- `{"op":"delete","node":"<id>","rev":<rev>}` and `{"op":"move",...}` do what they say.

`rev` is the node's revision as you know it: the `rev={n}` a read shows on the tag, and 1 when the read shows none. A stale `rev` is refused, nothing changes, and the error names the current one. For a node only you write, send the batch again with it. For a node another agent may have changed, read it again first and redo your change on what is there now.

## Seeing what you drew

Tags that parse can still look wrong. Look at the image of every board you write.

```sh
omni --as <run>/<role> export --project <project id> --feature "<feature>" --out <scratch>/look-1
ls <scratch>/look-1/ui/*/*-<board id>.png
```

The command writes each board of the feature as `<board name>-<board id>.png` (at twice the board's size, in the design system's first mode) with its tags beside it. Open the images of your boards with the Read tool. `omni project list` and `omni link` print the project ID.

In each image, look for:

- text that is cut off, wraps where it should not, or runs outside its container;
- layers that overlap or touch;
- content that ends past the board's edge, and a board that is mostly empty;
- columns and edges that do not line up;
- a blank where a token, a font or a component failed to resolve.

What shows in the image is real, whatever the tags say. Fix it, write the look to a new folder (`look-2`) and check again. Three looks at most; report what still fails after the third.

## Prototype: interactions and flows

A layer moves the person on through `interactions`, an attribute like any other (`insert`, `set` and `replace` write it):

```
<Frame name="Open round" ... interactions={[{trigger: {type: "onClick"}, actions: [{type: "navigate", dest: "<board id>"}], transition: {type: "slideIn", direction: "left"}, durationMs: 300, easing: "easeOut"}]}/>
```

- **Triggers:** `onClick` (the default), `onHover`, `onPress`, `mouseEnter`, `mouseLeave`, `afterDelay` (`ms`), `keyPress` (`key`).
- **Actions:** `navigate` (`dest`, a board), `back`, `openOverlay` (`dest`, a board; `position` such as `center` or `bottomCenter`; `background`, a dim colour written as `#rrggbbaa`; `closeOnOutside`), `closeOverlay` (on a layer inside the overlay board), `swapOverlay`, `scrollTo`, `setVariable`, `conditional`.
- **Transitions:** `instant` (the default), `dissolve`, `smartAnimate`, and `moveIn`, `moveOut`, `push`, `slideIn`, `slideOut` with a `direction` (`left`, `right`, `top`, `bottom`). `durationMs` and `easing` (`easeOut`, `easeInOut`, `linear`, or `{type: "customBezier", x1, y1, x2, y2}`) shape it.
- An interaction sits on the layer the person touches: the frame or the instance placed on the board.

A page lists where its prototypes start. Set `flows` on the page with a `set` operation: `{"op":"set","node":"<page id>","rev":<rev>,"properties":{"flows":[{"name":"Log a round","start":"<board id>","description":"From the home screen to the saved round"}]}}`. The list you send replaces the page's flows, so include the ones that stay (`omni design flows` prints them). Flow names are unique on a page.

`omni --as <run>/<role> design flows --doc <doc>` prints the graph: each flow, each board's interactions with their targets, the boards nothing reaches (`unreachable`) and targets that no longer exist (`dangling`). A prototype is wired when it reports no dangling target and no unreachable board other than flow starts and overlays opened by an interaction.

## Comments

`omni --as <run>/<role> comment list --doc <doc> --json` lists the human's comments. Each has `node` (a layer's ID, or `<instance id>/<component node id>`), `text`, `replies` and `waiting_on_you`. A comment is open while `waiting_on_you` is false. To find its board, read the doc (`omni doc read <doc>`) and take the `<Board>` whose tags hold that node ID. After acting on a comment, reply with what changed: `omni --as <run>/<role> comment reply <comment id> "<text>"`.
