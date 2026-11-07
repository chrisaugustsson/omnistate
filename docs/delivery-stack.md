# The delivery stack

A delivery run (`make deliver FEATURE=...`, the `omni-deliver` workflow) reads
its board from the OmniState instance on this machine and, in its test lane,
builds and tests every merged story. Those are two different compose projects
of the same `docker-compose.yml`:

| Project | Address | Holds | Started by |
| --- | --- | --- | --- |
| `omnistate` | http://omnistate.localhost | the board the run claims stories from, and the human's data | `docker compose up` (never a run) |
| `omnistate-test` | http://omnistate-test.localhost | the integration branch's build, and whatever the stack tests wrote | the lane's `stack_up` command |

Both route through the edge (`docker/edge/README.md`); without it they are
http://localhost:8420 and http://localhost:8421. Each project has volumes of its
own (`omnistate-test_postgres-data`, `omnistate-test_rustfs-data`,
`omnistate-test_test-target`, `omnistate-test_test-cargo-registry`), so nothing
a run builds, migrates or writes reaches the board.

## What the lane runs

The commands come from `[checks]` in `.omni.toml` (`omni checks --json`), and
run from the main worktree after the story is merged onto the integration
branch:

- `stack_up`: `OMNI_PORT=8421 docker compose -p omnistate-test up --build --wait --detach proxy`
- `stack`: `OMNI_PORT=8421 docker compose -p omnistate-test run --rm test`

The same two steps are `make test-stack-up` and `make test-stack` for running
them by hand. The lane runs the fast command first and `stack_up` only after
it passed: the web image copies `web/public/render`, which the fast command's
`make render-wasm` rebuilds, so a stack built at the same time served a stale
renderer and the verifier checked the wrong build. The verifier that exercises the story is told which stack
`stack_up` started and checks the story against that one.

Every command the lane runs has a time limit: 45 minutes, unless `timeouts`
in the run's args sets another for its name, in minutes (`fast`, `stack_up`,
`stack`, and `check` for the test author's command), as in
`--args '{"timeouts":{"stack":60}}'`. A command still running at its limit is
stopped with everything it started (its process group gets SIGTERM, then
SIGKILL 10 s later) and counts as failed; its output, which the finding
quotes, ends with the limit it ran past.

When a lane fails after `stack_up` ran (the stack did not start, the stack
command failed, a criterion failed, the test author found a defect, or the
verifier returned nothing), the run resets the integration branch to the lane's
base and runs `stack_up` again from that tree, logging
`lane:<key>: rebuilding the stack from <branch> at <sha>`, so the test stack is
left on the integration branch's head rather than on the story it sent back.
A lane that fails at the fast command never started the stack and does not
rebuild it.

After the verifier, the test author writes the story's regression tests on
the main worktree and returns the command that runs them; the lane runs that
command on its test commit. A red run drops the test commits, the story stays
done on its verified implementation, and the report's Tests section says
`tests dropped`.

## A credential on the test stack

The test stack starts with no users, and the permission classifier refuses an
agent that writes into its database by hand. A verifier that has to call the
API or open the app there gets a credential with

```bash
make test-stack-seed NAME=us26
```

from the main worktree, which inserts a workspace, a user `<NAME>@example.com`
with a live session and a membership, mints an API key through the API, and
prints three lines: `workspace=<id>`, `session=<token>` and `api_key=<secret>`.
A human route takes the cookie `omni_session=<token>`; an agent route, and the
omni CLI with `OMNI_SERVER=http://localhost:8421`, take
`Authorization: Bearer <secret>`. The rows live in the test database until
`make test-stack-down` removes its volumes.

## Bringing the test stack down

```bash
make test-stack-down
```

which is `docker compose -p omnistate-test down --volumes`. The next lane story
(or `make test-stack-up`) creates it again from scratch; the first build after
that compiles the test binaries into a fresh `test-target` volume, which takes
a while.

The API applies its migrations on start, and `docker compose up` on the test
project does not undo one. A migration from a story the lane sent back stays
applied to the test database, even though the branch was reset and the rebuilt
API no longer carries it, until the command above removes the volumes. A story
that reads from a migration-created table and then fails to start because the
table already exists, or passes only because a sent-back story's table is still
there, is the sign to run it.

## Where a story's worktree starts

While the lane holds a merge, main carries a story that may yet be sent back
and reset away. A worktree cut from that head would carry the story's commits
and replay them at its own merge, so a new worktree starts from the lane's
base (the last head that passed) while the lane holds a merge, and the commit
it was cut from is recorded on the branch as `branch.story/<key>.omnibase`.
The lane rebases a story with `--onto main <that base>`, falling back to git's
fork point and then the merge base for a branch cut by an older run, so only
the story's own commits reach main. The recorded base counts only while the
branch still sits on it: when an implementer has rebased the branch onto main
itself (after a rebase conflict), the lane rebases from the merge base
instead, and a base that is on neither main nor the branch is ignored. Once the lane has rebased a story, the
recorded base moves to the head it was rebased onto, so a story the lane sends
back and merges again replays only its own commits.

A story that review or the lane sends back keeps its worktree and branch: the
next implementer continues from them and fixes the findings. Only a story that
passes the lane loses its worktree and branch.
