---
name: omni-behaviour
description: Grill a feature's behaviour tree into OmniState, user stories first, then every situation down to each outcome the user can see. Each run is one pass; the human answers the open questions on the Behaviour screen and runs it again. Use after omni-grill when the human wants a feature's behaviour worked out, or says 'omni-behaviour'.
---

Fill a feature's **behaviour tree** in OmniState: what the product does in each situation, down to every outcome the user can see. The point is to surface the edge cases (a name that is taken, a user logged out mid-save, a second press) before any screen or code exists. The tree's model and commands are in the `omni` skill's `behaviour.md`; read it first.

One run of this skill is one **pass**. A pass builds everything it can, leaves each real product choice as an open question on its node, and ends with a report. The human answers on the feature's Behaviour screen and starts the next pass with this skill. A feature takes several passes.

A pass asks the human one thing in the chat: the user stories, on the first pass. Every other question goes on a node.

You are the **dispatcher**. Each node is grilled by its own fresh node agent, which reads only what lies above that node and writes straight to OmniState. You own the root and the list of stories, you dispatch, and you write the report.

## Start

1. Read the feature from `OMNI_FEATURE`; without it, ask the human which feature. Follow the Start steps of the `omni` skill. Pick a run name for this pass, such as `behaviour-3`: you write as `<run>/grill`, node agents as `<run>/node`, the review agent as `<run>/review`.
2. Find the feature's Spec doc: `omni doc list --feature "<name>" --pillar spec`. Without one, stop: tell the human the grill builds on the Spec doc, which `omni-grill` writes.
3. Find the Project spec: `omni doc list --global --pillar spec`, title `Project spec`. A project may have none.
4. Read the tree: `omni behaviour list --json`.
5. Tell the human what you found, in one or two lines: "Spec doc: 9 decisions. Project spec: none. Tree: 34 nodes, 3 marked for another pass, 2 questions waiting for you."

An empty tree: go to Stories. Otherwise: go to The pass.

## Stories

Read the Spec doc and the Project spec. A **user story** is one role doing one flow for one purpose: "As a member, I want to rename a project so that its name fits what it has become." Take the stories from the Spec's goals and decisions, and propose the ones it implies without naming, such as the same change made through a second door (the web app, the command line).

Put them to the human in one round: a numbered list of story sentences, each with your recommendation (in, or out with the reason), worded so that "yes" accepts the list. The human confirms, corrects or adds. Ask again only about what they changed.

Then write them:

- Each story: `omni behaviour add --title "<story sentence>" --slug <the flow, kebab-case>`. The first add creates the root.
- A story the human left out: add it, set it aside with `omni behaviour edit <story> --status out-of-scope`, and record why with `omni behaviour ask <story> "Is this story in scope?" --answer "<the human's reason>" --by human`.
- The root: `omni behaviour edit . --summary "<one sentence: what the feature lets whom do>" --status done`. Its other fields stay empty; the Spec doc holds what the feature is.

Done when every story the human confirmed is a node under the root.

## The pass

Repeat:

1. `omni behaviour list --json`. A node is **work** when it is marked (`needs_pass`) or is `open` with `open_questions` at 0.
   - An open node with an open question and no mark waits for the human: leave it.
   - So does an open node with an answer that starts `Follows`, as long as the question that answer quotes is still open on another node (`omni behaviour questions --json`): leave it. Once the human has answered that question, the node is work.
   - A node that is `out_of_scope`, or lies under such a node, is never work. Clear a mark on it with `omni behaviour edit <path> --grilled`.
   - The root is yours: see below.
2. No work left: go to Review.
3. Take the work on the shallowest level that has any and dispatch one node agent per node, at most 10 running at once. Shallowest first, so a node is reworked before the nodes below it.
4. Wait for every `NODE DONE` report of that level and keep the report lines for the end report.

A node that is still work after two agents have reported on it in this pass is **stuck**: keep it for the report and leave it out of the rest of the pass.

**The root.** When the root is marked, a person wrote on it. Read it with `omni behaviour read . --json`. An open question there with `asked_by.kind` `human` is a remark about the stories: carry it out (add a story, set one out of scope), answer it with what you did (`omni behaviour answer <question ID> "Done: ..."`), then close it again with `omni behaviour edit . --status done --grilled`.

## Dispatching a node agent

Spawn one subagent per node with this brief:

> You are an omni-behaviour node agent. First read `<absolute path of node-agent.md in this skill's base directory>` and follow it.
> Node: `<the node's path>`
> Feature: `<feature ID>`
> Run: `<run>`
> Spec doc: `<doc ID>`
> Project spec: `<doc ID>` or `none`

A node the review sent to rework for a rule (see Review) gets one more line: `> Rule: <rule ID>`.

Add nothing else about the tree: the node agent reads its own context, and the brief stays the same size for every node.

## Review

When no work is left, spawn one subagent:

> You are an omni-behaviour review agent. First read `<absolute path of review-agent.md in this skill's base directory>` and follow it.
> Feature: `<feature ID>`
> Run: `<run>`
> Spec doc: `<doc ID>`
> Project spec: `<doc ID>` or `none`

Wait for its `REVIEW DONE` report. Its findings are open questions on nodes now; they wait for the human like any other.

Its **Rework** lines name nodes that a rule reaches and that do not follow it yet. The rule already decides those, so the human is not asked: dispatch a node agent for each, with the `Rule:` line, then repeat steps 1, 3 and 4 of The pass until no work is left, since a new child may be open. Go to Report without a second review; the next pass's review reads the result.

## Report

End the pass with a report to the human, from a last `omni behaviour list --json` and the lines the agents reported:

- The tree: how many nodes are done, open and out of scope, and how many questions wait for the human, with the link to the Behaviour screen: `<server>/w/<workspace_id>/projects/<project_id>/features/<feature_id>/behaviour` (`omni config show` has the server and the workspace).
- **By default**: of the answers agents gave on their own in this pass, the ones that change the product most, five at the most, each with its node path.
- **Spec**: every Spec decision the tree no longer follows, with the node. The Spec doc is the human's to change.
- **Product-wide**: every rule an agent reported as holding beyond this feature. Moving one to the Project spec is the human's call.
- Stuck nodes, when there are any.
- What comes next: with questions waiting, "Answer on the Behaviour screen, then run omni-behaviour again." With none waiting and no node open, say that nothing waits. Whether the tree is good enough to go on to the UI step is the human's call.

Leave out a part that has nothing in it.
