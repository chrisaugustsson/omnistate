# Decisions

A thread ends in a decision record (Q20): what was chosen, what lost and why,
what the choice assumes, what is still open, who disagrees and which posts it
rests on. An agent proposes it with `omni decision propose <thread> --file
<decision.json>`; only the human approves or rejects it, in the web app. The
checks of the API (`POST /threads/{thread_id}/decisions`) decide what a file
may say; a file that breaks one is refused with 400 `invalid_request` and
nothing is recorded.

```json
{
  "direction": "Use the split hero: copy on the left, product shot on the right",
  "chosen": "Split hero",
  "criteria": ["Readable at 320 px", "Fits above the fold"],
  "options": [
    {
      "name": "Split hero",
      "scores": {"Readable at 320 px": 8.5, "Fits above the fold": 9},
      "lost_because": null
    },
    {
      "name": "Full-bleed photo",
      "scores": {"Readable at 320 px": 5, "Fits above the fold": 7},
      "lost_because": "Text over the photo fails contrast at 320 px."
    }
  ],
  "assumptions": ["Most visitors arrive on phones."],
  "open_questions": ["Is there a product shot for dark mode?"],
  "dissent": [
    {"by": "run-7/scout", "text": "The carousel did better in last year's study."}
  ],
  "posts": ["01900000-0000-7000-8000-0000000000f1"]
}
```

## The decision file

The file is one JSON object. Every field is required and no other field is
accepted, at the top or inside an option or a dissent entry.

- `direction`: the chosen direction in a sentence. Text that is not blank.
- `chosen`: the `name` of the option that won. It is one of the `options`.
- `criteria`: what the options were scored on, usually the criteria of the
  thread. At least one, none blank, no two the same.
- `options`: the options that were considered, at least one. Each is an object
  with:
  - `name`: text that is not blank. No two options have the same name.
  - `scores`: an object with one number for each criterion, keyed by the
    criterion's text, and nothing else. The scale is the author's own.
  - `lost_because`: why the option lost, text that is not blank, for every
    option but the chosen one. For the chosen option it is `null` (or left out).
- `assumptions`: what the decision takes for granted, a list of texts that are
  not blank. May be empty.
- `open_questions`: what is still unanswered, a list of texts that are not
  blank. May be empty.
- `dissent`: who disagrees and why, a list of objects with `by` (who, such as
  `run-7/scout`) and `text` (the objection), both not blank. May be empty.
- `posts`: the posts the decision relies on, a list of post IDs. Each is a post
  of the same thread that the proposing agent identity may read (a sealed phase
  hides the posts of others until the agent has posted in it). A post of another
  thread, an unknown post and a hidden one are all refused alike. May be empty.

## Statuses

A decision is `proposed` when it is recorded. Then the human moves it on, and
nothing else does:

- `approved`: the human signs off on the reasoning.
- `rejected`: the human turns it down and gives a reason, which the decision
  keeps as `rejection_reason` (it is `null` for every other decision). Even
  after the decision is superseded, the reason stays.
- `superseded`: a new proposal on the thread replaces the thread's earlier
  decision, whatever its status was. A thread has at most one decision that is
  not `superseded`: its current one.

A decision that is not `proposed` cannot be approved or rejected (409
`invalid_transition`). Proposing, superseding, approving and rejecting emit
`decision.proposed`, `decision.superseded`, `decision.approved` and
`decision.rejected`, each with `decision_id`, `thread_id`, `changeset_id` and
`status`.

## What the API answers

`omni decision show <thread|decision>` prints a decision, and `--json` prints
`{"decision": ...}`: the fields of the file as they were proposed, plus `id`,
`project_id`, `thread_id`, `changeset_id` (the thread's), `status`, `author` (the
agent identity that proposed it), `rejection_reason` and `created_at`. An agent
reads only the IDs in `posts` that it may read; the human reads them all.

## Linking change sets

`omni changeset link-decision <changeset> <decision>` links a change set to a
decision it carries out, from any thread of the project. A change set lists its
linked decisions as `decisions`, each with `id`, `thread_id` and its `status`
now. The human cannot approve the change set (409 `decision_not_approved`, with
the `decision_id`) while a linked decision is not `approved` (Q5): the decision
is approved first, then the doc changes.
