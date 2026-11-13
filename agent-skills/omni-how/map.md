# The map

The map is what the human reads first: cards for the feature's screens, operations, tables and outside services, wires for what calls what, and one playable flow per user story. It shows the feature and the existing parts it touches, never the whole app. It is a page of the `system-map` skill, stored in OmniState as an html doc.

Read the `system-map` skill first. Its engine is finished and you write data. Follow its steps 2 to 5 (copy the template, write the data, lay out, verify in a browser) with what this file adds:

- Its step 1 is done: your inventory is the `Operations` and `Tables` docs and what the code reading found.
- Its step 6 is replaced by [Into OmniState](#into-omnistate).
- Work on `map.html` in a temporary folder outside the project. A copy the browser needs inside the project is deleted as soon as it is checked.
- Without a browser tool, check the data by reading (every wire's `a` and `b` and every flow's `route` and `nodes` name an id that exists) and say in the report that the map was not played.

Without the `system-map` skill installed, draw no map and say so in the report, with the links to the two docs.

## Data

**Cards.** One per part named in the two docs:

| Part | `kind` | `title` |
|---|---|---|
| Screen or command (one card per screen, whatever its states) | `browser` | the screen's name, from the design or else the tree |
| Operation | `service` | its name in `Operations` |
| Table | `db` | the table's name |
| Outside service | `cloud`, with `ext: 1` | its name |

- `id`: kebab-case. A part listed under `Builds on` or `Used as they are` gets an id that starts with `x-`: the page draws those cards **faint**. The feature's own parts, new and changed, are drawn at full strength, also on a rerun after they are built.
- `sub`: `new`, `changed: <what>` or `exists`.
- `about`: the part's one line, and `Choice C<n>: <the pick>` for each choice that concerns it.

**Zones.** `Screens` (`Screens and commands` with a command among them), `Operations` and `Tables` from top to bottom, and `Outside` (`cls: 'ext'`) to the right of `Operations`. Leave out an empty zone. Put an operation under the screen that calls it and a table under the operation that writes it; a table that most operations use can be one wide card under all of them. At most five cards in a row: the doc viewer is narrower than a browser tab and the page scales the map to fit.

**Wires.** Screen → operation for each `Called from`; operation → table and operation → outside service for each `Uses`; the same for the faint cards from `Used by` and from what the code reading found. The `label` says what passes: `read`, `insert`, `send confirmation`.

**Flows.** One per user story in scope, keyed by the story's slug, with the story in a few words as `name` (`Car owner books a time`). Its steps follow the story's main path through the tree, from the user's first action to the outcome they see, each `text` in the tree's words. A step that happens on one screen uses `at`. A side branch that ends somewhere else is one sentence in the step where it leaves, and that step's `nodes` include the cards only the branch reaches. Without a tree: one flow per thing the Spec's `Goals` say the user can do.

A new or changed card that no flow lights is a finding: an operation no story needs, or a step you left out of a flow. Settle it in the docs or the flow before you go on.

**Header and caption.** Eyebrow `How · <feature>`, the feature's name as `h1`, a lede of one or two sentences. The caption says what the feature adds and changes, in counts, that fainter cards exist today and stay as they are, and that the outline is in the docs `Operations` and `Tables`. The map's `id` is `how-` and the feature's name in kebab-case.

**`MAP:BODY`** holds exactly this block. It fits the page to OmniState's doc viewer and draws the `x-` cards faint:

```html
<style>
body { padding: 0 2.75rem 0 0; }
.omni-doc { max-width: none; color: var(--fg); }
.omni-doc #map-figure [data-omni-id].is-hover { outline: 0; background-image: none; }
figure { padding: 0; border: 0; text-align: left; }
.canvas svg { min-width: 0; }
.dia:not(.flowing):not(.tracing) .node[data-id^="x-"] { opacity: .5; }
</style>
<script>document.documentElement.dataset.theme = 'light';</script>
```

## Into OmniState

1. The doc's content is the page from its `<style>` to its end, inside one `<div>`, with the engine held back until the viewer's frame has a size. The viewer can run a doc's scripts while its frame is still hidden and 0 wide; the engine measures text, so started then it draws a blank map.

   ```
   { echo '<div>'
     sed -n '/^<style>/,$p' map.html | sed 's/<script id="map-engine">/<script id="map-engine" type="text\/plain">/'
     echo '<script>(function start(){if(!innerWidth)return setTimeout(start,50);var held=document.getElementById("map-engine"),run=document.createElement("script");run.textContent=held.textContent;held.replaceWith(run);})();</script>'
     echo '</div>'
   } > map-doc.html
   ```

   The title and the font links above the `<style>` stay out: the viewer has no network, and the page falls back to system fonts.

2. Write it.
   - The feature has no `Map` doc: `omni doc create --feature "<name>" --kind html --pillar architecture --title "Map"`, then

     ```
     jq -Rs '[{op:"insert",parent:null,position:{after:null},content:.}]' map-doc.html | omni doc edit <doc> --body-file - --as <run>/how
     ```

   - It has one: the doc keeps its ID and its link. `omni doc read <doc> | head -1` shows the root `<div>` with its `data-omni-id` and `data-rev`; replace that node:

     ```
     jq -Rs '[{op:"replace",node:"<id>",rev:<rev>,content:.}]' map-doc.html | omni doc edit <doc> --body-file - --as <run>/how
     ```

3. Check that `omni doc read <doc> | grep -c '<script id="map-'` prints `2`: the data and the engine both arrived. With a browser signed in to OmniState, open the doc's link in a new tab, see that the map is drawn, and play one flow there; otherwise the report says the map was checked outside OmniState only.
4. Delete `map.html` and `map-doc.html`. The map lives in OmniState only.

## For the report

Tell the human on a first run:

- A click on the map selects that block for a comment, which suits a remark about the map as a whole. A remark about one operation, table or choice goes on its row or block in the two docs.
- A layout they drag is gone on the next visit. `Copy layout` shows the positions as text; when they paste it to you, set each card's `x` and `y` from it and write the map again.
