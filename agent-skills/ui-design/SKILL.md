---
name: ui-design
description: The finished design of a feature on OmniState design boards, one board per screen or state, on the project's design system (created first when the project has none), with every value a token and every repeated element a component, wired into a prototype and checked on board images. Use after the human picked a layout option from ux-design, when asked to make wireframes look finished, or to rework finished boards from comments.
---

# UI design

The layout is decided: the human picked one of `ux-design`'s options, or combined them. This skill draws the feature once, as the finished design, and wires the boards into the prototype the human tries.

It leans on these; read each when its step comes:

- `ux-design`'s references: [boards.md](../ux-design/references/boards.md) (writing boards, looking at their images, interactions and flows), [rules.md](../ux-design/references/rules.md), [flows.md](../ux-design/references/flows.md) section 5 (the situation checklist) and [judging.md](../ux-design/references/judging.md).
- `design-taste-frontend`: `references/app-screens.md`, the house rules for screens people work in, and `scripts/draw.mjs --app`, its style deal.
- `emil-design-eng`, through [references/motion.md](references/motion.md).
- [references/notes.md](references/notes.md): the design notes, which link boards to behaviour nodes.

A caller such as a workflow may run the parts of a step (the design system, one screen, one user story, one review) as separate agents. An agent given one part does that part only.

## Inputs

- **The pick** (required for a first design): the human's words, and the "Layout options" design doc it refers to.
- **The behaviour:** the feature's behaviour tree when it has one, otherwise its spec.
- **The design system,** when the project has one.
- **Research and brand notes,** when there are any.
- **Destination:** a design doc on the feature, pillar `ui`, titled "Design", with one page; and the html doc "Design notes".

## Tokens only

On a finished board every colour, text measure, spacing, radius and shadow is a reference to a token of the design system (`fill="$color.surface"`), and a repeated element is an `<Instance>` of a component. So a theme is a mode of a token collection: one edit in the design system and every board follows.

- **A value the system lacks** is added there first, then used: `omni tokens set <path> <value> --type <color|dimension|number|string>` (a colour once per mode: `--mode light`, `--mode dark`). A shadow goes in with the whole tree: `omni tokens export --out <file>`, add it, `omni tokens import <file>`.
- **One writer at a time.** Every token write and every component write sends the whole tree, so two agents writing at once lose each other's work. An agent that draws beside others writes neither: it uses the closest token, or builds the element from frames on tokens, and reports what the system lacked. The agent given the design-system part adds it afterwards and puts it on the layers.
- **Text:** `fontFamily` takes a string token, `fontSize`, `fontWeight` and `lineHeight` number tokens (`fontSize="$type.size.body"`). The system's `typography` tokens describe whole text styles for the code; a board refers to the single values.
- **A shadow** is referred to whole: `shadows={["$shadow.pop"]}`.
- **Sizes** (`width`, `height`) and `x`, `y` are numbers.
- **The one literal colour** is the scrim behind an overlay, inside an interaction (`references/motion.md`).

## Step 1: The design system

Boards take every value from the project's design system, so the system has to describe the product as it is.

### The design system and the product's code

When the folder linked to the project holds the product's code, its style values say what the product really looks like: the CSS custom properties (`--background`, `--sidebar`), a theme file, a tokens file. Find them, and compare them with the design system **by role**: for each thing the code styles (the page's background, the sidebar, a card, a border, body text, muted text, the primary action, a control's radius, a panel's shadow, the typeface, the body text size), find the token the design system gives that role (its name and `$description` say) and compare the two values, in every mode. Convert colour notations (oklch, hsl) to hex first; a difference under 2 in each of red, green and blue is none.

- **They agree:** go on.
- **They differ:** which one holds is the human's decision. Change neither the design system nor the code. Write the differences to the app-level html doc "Design system and code" (`omni doc create --global --kind html --pillar ui --title "Design system and code"`): the files compared, then a table with Role, Token, Design system, Code (the value and its name in the code), one row per role and mode. Roles only one side has go in a list under the table and do not count as differences. Report the differences and stop.
- **The project has no design system and the code has style values:** the new system is built from those values, role by role, in the token groups below.
- **No code in the folder, or no style values in it:** there is nothing to compare; say so.

### The system itself

**The project has one:** it is the look. `omni tokens export` prints its tokens; `omni doc read <design system doc>` prints the token tree and then its components as `<Component>` tags.

**The project has none:** create it before any board. With style values from the product's code, take the values from there and skip point 1.

1. Choose the style as section 2.3 of `app-screens.md` says: deal the options with `draw.mjs --app --kind <kind> --brief "<product>" --screens <screens> --options 3`, choose the one that fits how the product is used, and deal its sheet with `--style <id> --out <scratch>/taste`. Write one line on why. Without Node, start from the default tokens in section 9.5 of that file.
2. Write the tokens as DTCG JSON: `color` (type `color`) with the modes `light` and `dark`; `spacing` and `radius` (type `dimension`, values such as `"16px"`); `shadow` (type `shadow`); `type` with `family` (strings), `size`, `weight` and `line` (numbers; a line height is a multiple of the size); `motion` as `references/motion.md` lists. The root names the modes, and a token with a value per mode lists them:

   ```json
   {
     "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
     "color": {"$type": "color",
       "surface": {"$value": "#ffffff", "$extensions": {"omnistate": {"modes": {"light": "#ffffff", "dark": "#111111"}}}}},
     "spacing": {"$type": "dimension", "md": {"$value": "16px"}},
     "type": {"size": {"body": {"$type": "number", "$value": 14}}, "family": {"sans": {"$type": "string", "$value": "Inter"}}}
   }
   ```
3. `omni doc create --global --kind design-system --pillar ui --title "Design system"`, then `omni tokens import <file>`. A refused import names the token and the fix.

**Components.** List what the screens repeat (button, field, nav item, list row, card, pill, dialog shell) and define each the system lacks as a `<Component>`: small, built on tokens only, with `props` for what varies (`{type: "variant", values: [...], default: ...}`, `{type: "text", default: ...}`, `{type: "boolean", default: ...}`) and `bind` on the layer a property drives (`bind={{content: "label"}}`).

- Components are written all at once: `PUT /api/v1/w/<workspace id>/projects/<project id>/components` with the body `{"tags": "<all components>"}` replaces every component, so send the existing ones too, each with its `id`: `jq -Rs '{tags:.}' <scratch>/components.tags | omni api PUT <path> --body-file -`. `omni config show` prints the workspace ID.
- A shell (sidebar, top bar) is a frame of instances on each board, so each nav item can carry its own interaction.

Done when the system holds every token and component the planned screens need, and a board named `Components`, showing one instance of each component and variant, looks right as an image.

## Step 2: Plan the boards

From the pick and the behaviour, decide the screens, then per user story its boards.

- **One board per screen or state.** A screen's normal state is one board. A situation of the tree is a board of its own when the person sees something else than on a board already planned: a state of a screen (empty, an error under a field, a saved confirmation), or an overlay (dialog, sheet, menu, toast) as a small board.
- **One board per state,** across the whole design. Stories overlap: when two of them reach the same state of the same screen (a refused name while renaming, and again while fixing the name), one board shows it and both stories point to it. It belongs to the story a person meets first. So the boards of all stories are listed and compared before any is drawn, and every board name is unique.
- **A node that points to another** (`same-as`) is shown by that node's board. **A node whose outcome is off screen** (an email arrives) has no board; note why.
- **With a behaviour tree,** its situations are the list. Hold each screen against the situation checklist (`flows.md`, section 5) and add only what the screen cannot work without, each as an assumption.
- **Without one,** derive the situations from the spec with that checklist, the must-cover ones first.
- **Names:** `<Screen>` for a normal state, `<Screen> · <state>` for the rest: `Sign in · wrong password`.

Done when every node of the story (or every must-cover situation) has a board, points to one, or has a reason for none.

## Step 3: Draw

- Follow "Tokens only" above, the house rules of `app-screens.md` (sections 4 to 13, for the kinds of screen you draw), and `rules.md`.
- Real copy in the users' words, realistic data, the longest realistic values where a situation is about overflow.
- A state starts from its screen's normal board: read that board's tags, keep everything the state does not change, and write the result as a new board.
- Each state reads at a glance and without relying on colour alone.
- Look at the image of every board you write (`boards.md`).

Done when every planned board exists, its image passes the list in `boards.md`, and it holds no literal colour, text measure, spacing, radius or shadow.

## Step 4: Wire the prototype

- Each control that leads somewhere carries the interaction of `references/motion.md` for that moment. To add one to a layer that has interactions already, read the layer and write its whole list with yours added.
- A screen's normal board is shared by every story that uses the screen. When stories are drawn side by side, one agent writes the interactions on those boards, from what each story's agent reports.
- The page has one flow per user story, starting at the board where the story begins.
- A state the story's main path cannot reach (an error, no connection, a session that ran out) gets a flow of its own, named `<Story>: <situation>` and starting at that board; its controls lead back into the main path.

Done when `omni design flows --doc <doc>` shows no dangling target, and no unreachable board except `Components`.

## Step 5: Check

Review each user story on the images of its boards, as someone who did not draw them:

1. **Coverage:** every node of the story has its board, its pointer or its reason.
2. **Walkthrough** of the story on the boards, with the four questions of `judging.md`.
3. **Rule pass** with `rules.md`, its common AI mistakes first. Read *frame* as board and *Flows item* as a story or node of the tree; a rule that asks for an annotation is checked on the prototype's interactions.
4. **The UI-stage list** of `rules.md` ("Left for the UI stage"): contrast of text, controls and focus at WCAG AA, computed from the token values in every mode; text sizes; reflow.
5. **Taste:** the questions of section 2.7 of `app-screens.md` that an image can answer, and its banned list in section 13.
6. **Components:** an element that repeats across the story's boards is an instance of a component, not frames drawn again.

Rate each finding with the severity scale of `judging.md`, name its board and layer, and give the fix. Fix everything of severity 3 or 4; fix a 2 when the fix is one edit.

**The token check** runs last, over the whole design doc and the design system's components, and must come back empty:

```sh
omni --as <run>/<role> doc read <design doc> > <scratch>/design.tags
omni --as <run>/<role> doc read <design system doc> >> <scratch>/design.tags
grep -o -E '[A-Za-z]+[=:] ?\{?"#[0-9a-fA-F]{3,8}"' <scratch>/design.tags | sort | uniq -c
grep -o -E '(gap|pad|radius|fontSize|fontWeight|lineHeight|letterSpacing)=\{[^}"$]*[0-9][^}"$]*\}|fontFamily="[^$"][^"]*"|shadows=\{\[\{' <scratch>/design.tags | sort | uniq -c
```

The first lists literal colours with the attribute that holds them; `background` (the scrim of an overlay, the page's play-mode ground) and `content` (a colour code shown as text) are the only ones that may stay. The second lists literal text measures, spacing, radii and shadows, one attribute per line; a plain `{0}` may stay. Replace everything else with a token, adding the token first when the system lacks it, and run the check again.

## Step 6: The design notes

Write or update the "Design notes" doc as [references/notes.md](references/notes.md) describes: the last run, the motion for the code, and the table of boards and situations with each node's fingerprint.

## Reworking a finished design

After the human has tried the prototype, a rerun changes two kinds of boards and leaves the rest untouched:

- **Boards with an open comment** (`boards.md`, Comments). Do what the comment asks, in place, and reply to it with what changed.
- **Boards of situations that changed in the tree** since the notes were written (`references/notes.md`, "What a rerun reads from it"). Read the node again with its questions, and change what the change calls for; when it calls for nothing on the board, leave the board.

Change a board with `set` and `replace` on the layers that change, so IDs and comments stay. Then run steps 4 to 6 for the boards touched; the token check always covers the whole doc.

## Hand-over

Report the design doc and the notes doc, the boards drawn or changed, the design system's style and why (when it was created), what the checks found and what is still open, and the result of the token check.
