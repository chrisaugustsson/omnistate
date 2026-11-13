# Layout options: lenses and the grey format

## The three lenses

All three options serve the same user stories. They differ in how, and each decides its own screens.

- **A, Familiar** (new design): built from the strongest patterns in the research and from what users already know in similar products. With no research given, look up two or three comparable products when web search is available, otherwise build on common conventions for this kind of product. **A, Refine today** (redesign): keep the known structure and fix what the behaviour and the rules call for.
- **B, Task first:** the fewest steps and decisions for the most frequent story. Everything else moves to secondary views or behind disclosure.
- **C, Bold:** a clearly different interaction model from A and B, for example inline editing instead of forms, a board or canvas instead of a list, one adaptive screen instead of several. Bold serves every story and follows the rules like the others.

## What an option draws

An option is an overview: the layout of the feature, enough to choose between three.

- **Key screens only:** the screens a person passes through in the stories, six at most. A dialog or sheet is a board of its own only when it carries the option's idea.
- **The normal state:** realistic data, the main task possible. Empty, loading, error and the other situations come later, in the finished design. The exception is a situation that is itself one of the feature's jobs (a refused name, for a rename): each option shows how it handles that one.
- **One screen size:** the product's main one. The spec's when it names one; otherwise 1440 × 900 for a product used at a desk and 390 × 844 for one used on a phone.

## The grey format

- **Grey only,** as literal values from one ramp: `#111111`, `#444444`, `#888888`, `#cccccc`, `#eeeeee`, `#ffffff`. Hierarchy comes from size, weight, position and spacing.
- **One typeface:** the family of the project's design system when it has one, otherwise `Inter`.
- **Real copy** in the product's language and the users' words, with realistic data.
- **Board names** are `<letter><number> <Screen>`: `B2 Round detail`. The number follows the order a person meets the screens.
- **Placement:** the caller gives each option its column, an `x`. The option's boards stack downwards from `y` 0 with 120 between them.
- **A label above each column,** so a person looking at the whole page sees which boards belong together: a board as wide as the column, 320 high, 120 above the first screen, named `Option <letter>: <lens>`. It holds that same line at size 96 and weight 600 on `#111111`-on-white, and under it the option's one sentence at size 32. The label is a note: no interaction leads to it.
- **Clickable:** each control that leads to another key screen of the option carries a `navigate` interaction to that board, so the option's main path plays as a prototype.

How to write boards, see them as images and wire interactions: [boards.md](boards.md).

## Self-check of one option

Before handing over, go through these and fix what fails:

1. Every screen of the plan is a board, and every story has a place in the option where it happens.
2. Every board's image has been looked at and passes the list in `boards.md`.
3. Each board says what it is for with its title and its largest element.
4. Each board has at most one primary action, and every button is a verb naming its result.
5. No board shows a common AI mistake from [rules.md](rules.md) that applies to a normal state: emphasis in the wrong place, a primary button on every card, modals for everything, system words, decorative dashboards, a phone as a squeezed desktop.
6. Every `navigate` lands on a board of this option.

## The comparison note

One board named `Comparison`, 560 wide, left of the three columns, text only, grey like the options. One `Text` layer per item, so a comment can point at one item:

1. **Per option:** its lens, the one sentence that makes it different, where it is strongest and where it is weakest. Strength and weakness name a user story or a rule ID from `rules.md`, and what the user gains or loses.
2. **Recommendation:** one option, and the reason in two sentences at most.
3. **Worth combining:** a part of another option that would fit the recommended one, when there is one ("A's sidebar under B's week view").
4. **Unproven:** each pattern that users of this kind of product have not met before, with the option that uses it.
5. **Assumed:** what the run had to assume. With a behaviour tree: how many assumptions were added to it as questions answered by default, and that they are marked in the Behaviour screen. Without a tree: each assumption as one line of 20 words at most. Five or fewer go in the note; more go on a board of their own named `Assumptions`, under the note, and the note says how many there are and points there.
6. **Next:** "Pick one option, or combine parts of them, and start the finished design with that pick."

The note is short: a person reads it in two minutes. Per option that is three sentences: what it is, its strongest point, its weakest point.
