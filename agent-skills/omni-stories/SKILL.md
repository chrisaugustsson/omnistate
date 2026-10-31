---
name: omni-stories
description: Cut a feature's settled spec into vertical-slice stories and save them to the feature's board in OmniState, with dependencies, files, criteria and doc links. Use after omni-grill when the human wants the feature on the board, or says 'omni-stories'.
---

Turn the feature's settled Spec, Architecture, DB and UI docs into stories on its board, ready for `omni-deliver` to pick. You write stories only through `omni story add`; you write nothing under `tasks/`, and you do not ask for confirmation before saving: the human steers from the board afterwards.

## Start

1. Read `.omni.toml` for `project` and `feature`. Without a `feature`, ask the human which feature to cut.
2. Follow the Start steps of the `omni` skill (`omni whoami --as <run>/<role>`, `--as` on every command). Keep the `<run>` id; `omni story decision` needs it as `--run`.
3. Read the feature's docs: `omni doc list --feature "<name>"`, then `omni doc read <doc>` for the Spec, Architecture, DB and UI docs that exist. Note each doc's ID and the node IDs of the contract operations, tables and screens (`omni design read --doc <doc> --board <board>` for one board of a large design).
4. Read the repository for what each story changes: the closest existing code, tests and routes for every operation, table and screen.
5. `omni story list` shows the stories the feature already has. Do not cut a story twice; when the board already holds stories, continue after them.

## Cutting

Each story is a vertical slice through every layer it needs (schema, API, CLI, web), narrow but complete, and lands on a working app. Never cut "the whole API", "all the types" or "all the tests". A refactor that makes a later story easy is its own earlier story (a prefactor story, Topic kind `prefactor`). Order the stories so each lands on a working app.

- **Size.** At most 4 behaviour criteria per story, not counting the standard check line (`Typecheck/lint passes`) or the browser line. More than 4: split the story. If the change cannot be described in 2-3 sentences, it is too big.
- **Contract.** Each story names, in its Description, the contract operations (or tables, or screens) it implements, by their exact names from the frozen docs. A prefactor story that implements no operation names none.
- **Criteria** are verifiable: a concrete input and the exact expected value, status, error text or row count. "Works correctly" is not a criterion.
- **UI stories** carry the criterion `**[UI stories only]** Verify in browser using dev-browser skill`, written as `Verify in browser using dev-browser skill (<what the page shows>)`.
- **Interfaces** list the exact names and signatures the story adds that later stories use, and the existing ones it uses.
- **Dependencies.** A story that uses an Interface another story adds names that story in `Blocked by`. A story with no such dependency says `Blocked by: none`. Never block on order alone.
- **Files.** `change:` lists the paths the story changes, found by reading the repository, as backticked paths. `read first:` lists at most 6 paths, e.g. the closest existing test to copy. Never hardcode a sequence number that depends on story order (a migration `V003__`): write "the next free migration number".
- **Links.** Every story, prefactor stories included, links at least one Spec, Architecture or DB node it builds from with `--link <doc>:<node>`, and each UI story also links its design board. Repeat `--link` for more than one; `<doc>` and `<node>` are IDs, an optional `:<label>` follows.

## Story file

Write each story to a scratch file outside the repo (the session's scratch directory), in this shape:

```markdown
### US-<n>: <Title>
**Description:** As a <user>, I want <feature> so that <benefit>. Implements <contract operations>.
**Topic:** <kind: feature | prefactor | bugfix | chart | docs> · <1-3 domain tags>
**Blocked by:** <US-xxx, ... or none>
**Interfaces:** <names and signatures added for later stories, and existing ones used>
**Files:** change: `<path>`, `<path>` · read first: `<path>`

**Acceptance Criteria:**
- [ ] <behaviour criterion with the exact expected value>
- [ ] Typecheck/lint passes
- [ ] **[UI stories only]** Verify in browser using dev-browser skill
```

Use a `US-<n>` of your own in the heading for ordering and for `Blocked by`; the server assigns the real keys. After each add, note the key it printed and use the real keys in the `Blocked by` of later stories.

## The two-implementation test

Apply it to every acceptance criterion of every story before saving, and rewrite the story until no blocking gap remains.

```
## The two-implementation test (MUST apply to every acceptance criterion)
For each criterion, imagine two implementations a reasonable developer could build
that BEHAVE DIFFERENTLY. If both satisfy the criterion's wording, the criterion does
not decide the behaviour: it is a gap. Example: "an inverted limit gets a defined
result" is met both by rejecting with 422 and by accepting, so it is a gap; "fails
with HTTP 422 and the detail X" is not.
- Words like handled, supported, works, correctly, gracefully, appropriate, sensible,
  a defined result, as expected often mark a gap; the test above decides.
- A general principle elsewhere in the spec fills a gap only when it names this exact
  case. When you believe a principle points to an answer, it is still a gap: give
  that answer as the recommendation.
- Not a gap: a criterion with a concrete observable outcome, standard checks
  (typecheck, lint, build), and choices the spec leaves to the implementer on purpose
  (internal structure, names, which helper to use).
For each gap give the question, two or more concrete options, and your recommended
option with the spec text behind it.

## Blocking or minor (decide for every gap)
Judge each criterion as written, at the case it describes. Not every open case is
worth a stop: an answer to a minor question only makes the criterion longer, and the
next check finds a smaller edge in it.
- BLOCKING: the answer changes what a user sees or can do in the normal flow the
  story describes; what is stored, sent or deleted; an API contract (status, shape,
  error text); permissions, security or money; or the spec contradicts itself.
- MINOR: timing and concurrency (races, late or stale responses, requests in flight
  during unmount or while the input changes, overlapping retries); ordering,
  duplicates or normalisation of inputs the story does not name; cosmetic detail
  the spec does not fix; a case that only arises when two or more edge conditions
  combine; a case the codebase already handles by a convention a reasonable
  developer would follow.
- The test: if the spec author would accept either option shipped without noticing,
  it is minor. For a minor gap, recommend the most conventional, least surprising
  option; it will be built and verified as if the spec said so.
```

- A blocking gap: answer it from the Spec doc's Decisions table when it names the case, otherwise rewrite the criterion with the recommended answer when the spec points to it. When only the human can decide, do not guess: leave the story out of this cut, start a thread with `omni thread create "<question>" --feature "<name>"`, and list it in your hand-off.
- A minor gap: rewrite the criterion with the recommended answer, and after the add record it with `omni story decision <key> --run <run> --question "<question>" --choice "<answer>" --by agent`.

## Saving

Save the stories in dependency order, so every blocker exists before the story that names it:

```
omni story add <file> --link <doc>:<node> [--link <doc>:<node>:<label>]...
```

`omni story add` prints the story's key. Only for the project's very first story, and only when the human has given a number to continue from, pass `--number-from <n>`. Pass `--priority <n>` only when the spec says a story matters more or less than the rest (1 highest, 3 the default; 0 is reserved for fix stories). Fix a rejected add and retry; never save a story twice.

When all are saved, run `omni story list` and `omni story list --pickable`. The pickable list must hold exactly the stories with `Blocked by: none`; when it does not, name the story and the missing blocker in your hand-off: an agent key cannot edit a saved story, the human changes its blockers on the board.

## Done

Every contract operation, table and screen in the frozen docs is implemented by at least one story. Print the keys with their titles and the dependency order (which stories can start now, which wait for which), and the link to the board. Name each minor decision you recorded and each question you put in a thread. Running the board is the `omni-deliver` workflow's job.
