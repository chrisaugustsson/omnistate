# Flows: from spec to jobs and situations

How to read what users must be able to do, and every situation they can meet, out of a spec. A feature with a behaviour tree has this already: its user stories are the jobs and its nodes the situations. This file is for a feature without one, for `ux-review`, and for its situation checklist (section 5), which `ui-design` holds a screen against to find a state nobody described.

Every item records:

- **ID:** `ROLE-1`, `OBJ-1`, `JOB-1`, `TASK-1`, `SIT-1`.
- **Source:** the spec line, or *inferred* with the reasoning.
- **Priority:** *must-cover* or *should-cover* (section 6).
- **Done when:** one line a reviewer can check against a design.

## 1. Roles

Every role in the spec, and what each may see and do. Permissions decide which actions and situations each role meets.

## 2. Jobs

One job story per distinct circumstance: "When [situation], I want to [motivation], so I can [outcome]". Record actor, trigger, outcome, frequency (rare, occasional, frequent) and importance. Write the outcome as the progress the person wants ("so I can hand the file to my accountant"), one step beyond the action itself.

Find the steps the spec forgot with the job map: define, locate, prepare, confirm, execute, monitor, modify, conclude. *Monitor* (seeing how it is going) and *modify* (changing it afterwards) are the ones specs leave out most.

## 3. Objects

The spec's nouns become objects. For each object:

- **Relationships** to other objects, with cardinality, in both directions.
- **Actions** per role: who triggers it, where it starts, how often, how complex.
- **Attributes**, split into content and metadata. Metadata drives sorting, filtering and list rows.
- **Lifecycle states** the spec names or implies (draft, active, archived, deleted, locked).

Objects come before actions. Record as needs that each object has a place to browse many and a place to see one, and that each action starts from the object it acts on.

## 4. Top tasks

The frequent jobs and the critical ones. For each: goal, subtasks, steps, and at each step the decision the user makes and the information they need. Note variations: first use and repeat use, novice and expert, a missing precondition. A walkthrough goes through these step by step.

## 5. Situation sweep

For every object × action × role, go through the checklist and add each situation that applies as a `SIT` item, with the design response it needs and whether the spec states it or it is inferred. Record the sweep as a table: one row per object × action × role, the `SIT` IDs taken, and each situation skipped with its reason ("Offline: admin tool, spec says always online").

| Situation | What a design must show |
|---|---|
| Ideal | Typical, realistic data; the main task possible. |
| First-use empty | Nothing created yet: what will appear, one action that starts it, sample data where setup is heavy. |
| No results | A search or filter matched nothing: the query stays visible, a way to change or clear it. |
| Cleared empty | The user removed or finished everything: confirms it is done. |
| Loading | First load, loading more, refreshing; indicator chosen by duration. |
| Long operation | Determinate progress, an estimate, Cancel. |
| Pending action | The control or row is busy; double submission blocked. |
| Sparse | Very little data: guides toward the ideal state. |
| Partial failure | One region failed while others loaded: its own error and Retry. |
| Error, system | Could not load or save: cause if known, Retry, what happened to the user's data. |
| Error, validation | Messages at the fields, a summary when several, input kept. |
| Offline | Indicator, cached content usable, edits queued as pending, sync confirmed. |
| No permission | What is restricted, why, how to request access. |
| Not found | Deleted item or broken link: a route back. |
| Success | The result visible in context, Undo where it applies. |
| Unsaved changes | Save status or a changed marker, and a guard on leaving. |
| Conflict | Someone else changed it: who and what, review, choose a version. |
| Overflow | Longest strings, largest counts, many items, missing optional fields. |
| Timeout, session expired | A warning with one-action extend; sign in again with input kept. |
| Interrupted | The user leaves mid-task and returns: where they land, what was kept. |
| Lifecycle state | Each state of the object (draft, archived, locked, ...). |
| Selection | Items selected: count, bulk actions, a way out of selection. |

## 6. Priority

*Must-cover:* every top task, every core job, and every situation where getting it wrong loses data, money or access. Everything else is *should-cover*.

## Where it is written

The caller decides: `ux-design` keeps the jobs as a plain list in its plan, and `ux-review` writes them to its `tasks.md`. Each item's ID stays visible wherever findings cite it.
