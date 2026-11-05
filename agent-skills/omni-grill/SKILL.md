---
name: omni-grill
description: Grill the user about a feature and write the result into the feature's Spec doc in OmniState as you go, then settle and freeze the API contract and data model before stories are cut. Use when the user wants to shape a feature for the board, or says 'omni-grill'.
---

Interview the user the way `~/.claude/skills/grilling/SKILL.md` does, and keep the answers in OmniState instead of in the chat. Read that skill first and follow it: design tree, rounds that ask the whole frontier, numbered questions with a recommended answer each, facts found by sub-agents and never asked of the user. This skill adds where the answers go and what happens after the interview.

## Start

1. Read the feature from `OMNI_FEATURE`. Without it, ask the human which feature to grill, or create it with `omni feature create "<name>" --description "<what changes for whom>"`.
2. Follow the Start steps of the `omni` skill (`omni whoami --as <run>/<role>`, `--as` on every command).
3. Find the feature's Spec doc: `omni doc list --feature "<name>"`. When there is none, create it: `omni doc create --feature "<name>" --kind html --pillar spec --title "Spec"`.

## The Spec doc

Keep it in this shape (see the `omni` skill's `html-docs.md` for how to write nodes). All dates are ISO `YYYY-MM-DD`.

- `Goals`: what the feature is for and what the user can do afterwards.
- `Decisions`: a table with the columns `ID`, `Question`, `Decision`, `Date`. One row per settled question, numbered `D1`, `D2`, ... in the order settled, never renumbered.
- `Open questions`: the questions still open.
- `Non-goals`: what the feature leaves out.
- `Contract`: after `Non-goals`; holds the `Skipped: ...` lines (see below).

After every round, write the Spec doc: add a row per question the human settled, move settled questions out of `Open questions`, and add new open ones. Edit only the nodes that change (`insert` a table row, `replace` or `set` a list item) so the human's comments stay on nodes that still exist. Tell the human the link after each round.

## Behaviour part

Run rounds until the frontier is empty. Then end the part with a scope summary: goals, the decisions in short, non-goals, and whether the feature adds screens, endpoints and tables. The human confirms or corrects it; correct the Spec doc and ask again until confirmed. Do not start the contract part before the confirmation.

## Contract part

Two halves, in this order. Ask the human whether the feature adds endpoints, and whether it adds tables, when the summary did not already settle it.

1. **API contract.** When the feature adds endpoints, read the code for the conventions to follow, then write a `Contract` section in the feature's Architecture doc listing each operation with method, path, request, response, errors and access. When the feature has no Architecture doc, create it: `omni doc create --feature "<name>" --kind html --pillar architecture --title "Architecture"`.
2. **Data model.** When the feature adds tables, write a `Data model` section in the feature's DB doc with the tables, their columns and types, and the keys (primary, foreign, unique). When there is no DB doc, create it with `--pillar db --title "DB"`.

Show each half to the human and revise until they confirm it. Then end that section with a paragraph `Frozen for the build: <today's date>`. A frozen half is not edited by this skill again.

A half the feature does not need is skipped: leave that doc untouched and write in the Spec doc's `Contract` section the line `Skipped: no new endpoints` or `Skipped: no new tables`.

## Screens

When the feature adds screens, follow the wireframe and design-system recipes of the `omni` skill's `designs.md`: build a design doc in the `ui` pillar, reading the app level and its design system first. Record the chosen design's doc and board in the Spec doc's `Contract` section (`Design: <doc title>, board <board name>`, with the IDs). When the feature adds no screens, write `Skipped: no new screens` there.

## Done

The Spec doc holds `Goals`, `Decisions`, `Open questions` and `Non-goals`; the `Contract` section holds the design record or a `Skipped:` line for each of screens, endpoints and tables; every half that was not skipped is frozen in its doc. A question that only the human can decide and that blocks nothing else stays in `Open questions`. Hand off with the links to the Spec, Architecture and DB docs and the design. Cutting stories from the settled spec is the `omni-stories` skill's job.
