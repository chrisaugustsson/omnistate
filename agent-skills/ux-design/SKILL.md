---
name: ux-design
description: UX structure for a feature as three grey layout options of its key screens on OmniState design boards, with a comparison and a recommendation for the human to pick from. Use when turning a spec or a behaviour tree into screens, when wireframing, or when rethinking the layout of an existing screen. Structure only; the finished look is ui-design, and reviewing an existing design is ux-review.
---

# UX design

Decides where things go: which screens a feature has and how each is laid out. It widens: three options that differ, drawn as an overview, then a short comparison. The human picks; `ui-design` draws the pick as the finished design.

A caller such as a workflow may run the parts of a step (the plan, one option, the comparison) as separate agents. An agent given one part does that part only.

## Inputs

- **The behaviour** (required): the feature's behaviour tree when it has one (`omni behaviour list` prints every node, `omni behaviour read <node>` one node with its rules and questions), otherwise its spec. It decides what the product does.
- **Research,** when there is any: similar products and references.
- **For a redesign:** the current screens, code or screenshots.
- **The design system,** when the project has one: its component names go into labels, its typeface onto the boards.
- **Destination:** a design doc on the feature in OmniState, pillar `ui`, titled "Layout options", with one page for the three options. The caller names the doc and the page, or you create them (`omni doc create --feature "<feature>" --kind design --pillar ui --title "Layout options"`).

## Step 1: What the options must serve

- **With a behaviour tree:** the user stories (the nodes directly under the root) are the list. Per story note in one line what the user gets done and how often.
- **Without one:** derive the jobs from the spec as sections 2 and 4 of [references/flows.md](references/flows.md) describe, as a plain list.

Done when every story or job is on the list, with the most frequent one marked.

## Step 2: Plan three options

For each lens of [references/wireframes.md](references/wireframes.md) write: its key screens (six at most), the one sentence that makes it different, where each story happens, and each screen's layout in a few lines (regions, what is largest, where the main action sits). When two plans share a screen list, or a sentence fits another option too, rewrite until they differ.

Then write down what the three share, once: the parts of the screen the feature leaves as they are (navigation, the page around the change) and the sample data. The options are drawn apart, and a reader compares them only where they are meant to differ.

Done when the three plans differ in screens or in interaction model, and each gives every story a place.

## Step 3: Draw

Each option is drawn by its own subagent from its plan, so the three stay independent; an option's agent sees its own plan only. Without subagents, draw them one after another, each from its plan.

Follow the format and the self-check in `wireframes.md`, the rules in [references/rules.md](references/rules.md), and [references/boards.md](references/boards.md) for writing boards and looking at their images.

Done when each option has every screen of its plan as a board, each board's image has been looked at, and the option passes the self-check.

## Step 4: Compare

Look at the images of all three options side by side, then write the comparison note of `wireframes.md` as a board beside them, and set one flow per option on the page (`start`: the option's first board). Judge by what a user of each story meets, with the walkthrough questions in [references/judging.md](references/judging.md) for the most frequent story.

Done when the note names a recommendation with its reason, every option has a strongest and a weakest point tied to a story or a rule, and `omni design flows` shows one flow per option drawn and no dangling target.

## Assumptions

Where the behaviour is silent and the layout depends on the answer, choose what the rules and common conventions favour and keep going. A choice about stored data or permissions takes the most restrictive reasonable option.

- **With a behaviour tree,** record each choice on the node it belongs to, as a question already answered: `omni behaviour ask <node> "<question>" --answer "<the choice>"`. The human sees it marked as answered by default and can change it.
- **Without one,** list each choice in the comparison note.

## Hand-over

Report the doc and its page, the recommendation in one line, the assumptions made, and anything that still fails. The pick is the human's: stop here.
