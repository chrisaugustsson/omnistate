---
name: omni-grill
description: Grill the user about a feature and write the result into the feature's Spec doc in OmniState as you go. Use when the user wants to shape a feature for the board, or says 'omni-grill'.
---

Interview the user the way `~/.claude/skills/grilling/SKILL.md` does, and keep the answers in OmniState instead of in the chat. Read that skill first and follow it: design tree, rounds that ask the whole frontier, numbered questions with a recommended answer each, facts found by sub-agents and never asked of the user. This skill adds what the grill covers and where the answers go.

## Start

1. Read the feature from `OMNI_FEATURE`. Without it, ask the human which feature to grill, or create it with `omni feature create "<name>" --description "<what changes for whom>"`.
2. Follow the Start steps of the `omni` skill (`omni whoami --as <run>/<role>`, `--as` on every command).
3. Find the feature's Spec doc: `omni doc list --feature "<name>"`. When there is none, create it: `omni doc create --feature "<name>" --kind html --pillar spec --title "Spec"`.
4. Find the Project spec, the app-level doc `omni-init` writes: `omni doc list --global --pillar spec`, title `Project spec`. Tell the human in one line whether the project has one.
   - With one: read it. Its users, rules and stack are settled: build on them, and ask only what they leave unanswered.
   - With one, and the feature was created in step 1: add the feature to its `Features` list as the last item, with the name exactly as created and its one line, and take the feature's line out of `Not building` when it stands there.
   - Without one: the feature's Spec doc is all the grill needs.

## The Spec doc

Keep it in this shape (see the `omni` skill's `html-docs.md` for how to write nodes). All dates are ISO `YYYY-MM-DD`.

- `Goals`: what the feature is for and what the user can do afterwards.
- `Decisions`: a table with the columns `ID`, `Question`, `Decision`, `Date`. One row per settled question, numbered `D1`, `D2`, ... in the order settled, never renumbered.
- `Open questions`: the questions still open.
- `Non-goals`: what the feature leaves out.

After every round, write the Spec doc: add a row per question the human settled, move settled questions out of `Open questions`, and add new open ones. Edit only the nodes that change (`insert` a table row, `replace` or `set` a list item) so the human's comments stay on nodes that still exist. Tell the human the link after each round.

## The grill

The grill settles **what** the feature does, for whom and why. A question about how it is built (tables, endpoints, libraries) belongs to the how step, which comes after the behaviour and the UI are settled: say so and move on.

When an answer holds for every feature of the product ("everything is in Swedish and English"), ask the human whether it belongs in the Project spec. On a yes, insert it as a line under `Rules` there, in place of a row in this feature's `Decisions`. Otherwise, and in a project without a Project spec, it is a decision of this feature.

Run rounds until the frontier is empty. Then end with a scope summary: goals, the decisions in short, non-goals, and whether the feature adds screens. The human confirms or corrects it; correct the Spec doc and ask again until confirmed.

## Done

The Spec doc holds `Goals`, `Decisions`, `Open questions` and `Non-goals`, and the human has confirmed the scope summary. A question that only the human can decide and that blocks nothing else stays in `Open questions`. Hand off with the link to the Spec doc. Cutting stories from the settled spec is the `omni-stories` skill's job.
