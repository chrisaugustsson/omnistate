# Writing back to the how

For an agent that builds one task of a feature. `omni-how` wrote the feature's **how** into OmniState before the build. Two of its docs are yours to keep true: `Operations` (pillar `architecture`) and `Tables` (pillar `db`). Find them with `omni doc list --feature "<name>"`. A feature without them has no how, and this file asks nothing of you.

The how is a starting point. Build what the task needs, also where the docs say otherwise, and keep the docs true.

**Before you build**, read both docs. Read `Tables` again right before you add or change a table: another task may have written to it since.

**When your work lands**, in the same step, write to the docs. The step that runs you says when your work counts as landed.

- An operation you built: `set` its `Shape` cell to the exact shape: how it is called, what goes in, what comes back, how it fails.
- An operation you added, renamed or gave another job: `insert` or `replace` its row. One the feature no longer needs: `delete` its row.
- A table you built or changed differently from the doc: `replace` the column rows and key lines that differ, so the doc states the table as the code has it. A table the doc lacks: `insert` its section in the doc's shape. One the feature no longer needs: `delete` its section.
- An existing part you started to use, or stopped using: `insert` or `delete` its row under `Builds on` or `Used as they are`.
- A choice (a `Choice C<n>` block) you went against: `set` the block to what now holds and why, under the same ID.

Write the state as it now is ("status is booked, cancelled or done"). Edit only the nodes that change, each with the `rev` you read. A conflict means another task wrote first: read again and resend (the `omni` skill's `html-docs.md`).

Leave the `Map` doc as it is: the next run of `omni-how` draws a fresh one.

Done when, for every operation and table your task touched, the two docs say what the landed code does.
