# The delivery stack

A delivery run is the `omni-deliver` workflow (`workflows/omni-deliver.js`),
which Claude Code runs with its Workflow tool. It takes a feature's stories
from the OmniState board, implements, reviews and tests them, and lands them on
the branch `deliver/<feature slug>`. The script holds the flow, but it can run
no command itself: every git, omni and check command runs in its host helper,
`workflows/deliver-step.mjs`, which a cheap courier agent runs for each step.
The helper's header describes that protocol and every step; the script's
`meta` sums up a run.

A courier copies the helper's whole answer back, and a long answer does not
come back exactly: in the first real run, the courier of a story's context
failed four times on the hash check. So no courier carries story content, and
no answer the script reads passes about 2,000 characters, whatever a story
holds:

- The helper gives the script small facts, such as a story's attempt, how many
  criteria it has and which of them are test obligations, by index, and how
  many findings it has of each kind.
- Every agent reads its story itself, with
  `OMNI_RUN=<run> OMNI_ROLE=<role> OMNI_FEATURE=<feature> omni story read <key> --json`:
  its text, criteria, decisions, findings and linked doc nodes. Its prompt
  says how to tell the findings of this attempt budget, marked
  `[attempt N]` and open, from those that used no attempt and from those
  before the human's latest answer, and it reads the linked docs with
  `omni doc read`.
- An agent that needs a check's output reads the job's log by the path the
  prompt names. A finding or a thread about a check names the job, and the
  helper adds the end of its log.
- The helper writes the thread of a story out of attempts, and the report's
  list of blocked stories, from the board.

## Starting a run

Once per machine, `make install-agent-tools` links the workflow at
`~/.claude/workflows/omni-deliver.js` and its helper at
`~/.claude/omni/deliver-step.mjs`, the path the workflow runs it from. Both
links point into this checkout.

A run starts from a Claude Code session in the project's main checkout:

- The checkout's folder is linked to its OmniState project (`omni link`, or
  `OMNI_PROJECT`).
- Add the stories folder beside the checkout, `../<repo>-stories`
  (`../omnistate-stories` here), as a working directory of the session that
  runs the workflow: `claude --add-dir ../<repo>-stories`, or `/add-dir` in
  the session. The story worktrees and the lane worktree live there, and the
  run stops at its start when it cannot write there.
- A project with a heavy setup per worktree, such as dependencies to install
  or a long first build, warms the lane worktree, `<stories dir>/_lane`, before
  its first run: the first `fast` there starts cold, and has 45 minutes, the
  limit of a job, to finish. Make the worktree with
  `git worktree add --detach ../<repo>-stories/_lane`, then run the setup and
  `fast` in it once by hand. A run's start keeps that worktree, and its
  clean leaves ignored build caches in place.

Then ask Claude to run the workflow `omni-deliver` with the Workflow tool, with
args such as `{"feature": "Rename a project", "storyLimit": 3}`:

| Arg | Default | What it sets |
| --- | --- | --- |
| `feature` | required | the feature whose stories the run delivers |
| `implementers` | 3 | how many stories are implementing at once |
| `reviewers` | 3 | how many stories are in review at once |
| `storyLimit` | none | the most stories the run takes, for a first run on a feature |
| `force` | false | takes over the lock of a run the human knows is dead |
| `checks` | none | check commands on top of those in `.omni.toml` (see The checks) |
| `models` | see below | the model of a role: `clarity`, `council`, `impl`, `implRetry`, `review`, `verify`, `tests`, `final` or `courier` |
| `agentTypes` | none | the agent type of a role, such as `implementer`, where this machine has one |
| `helper` | `~/.claude/omni/deliver-step.mjs` | another path to the helper |

Every role runs on opus, except `impl`, a story's first attempt, on sonnet, and
`courier` on haiku.

The run never pushes, and never moves the checkout's branch or changes its
files: it reads the checkout's HEAD at its start, and keeps its own state in
`.omni/`, which git ignores.

## What a run does

- **Start.** The helper takes the lock, sets `deliver/<slug>`, reads the checks
  and opens the run's thread, `Run <run id>`. It releases the stories a dead
  run still holds, stops the jobs of earlier runs that still run, and cleans
  the lane worktree with `reset --hard` and `clean -fd`, never `-x`, so that
  ignored build caches stay warm.
- **Unblock.** A story whose threads all have a conclusion gets each one as a
  human decision, and is unblocked. Its attempts count from there.
- **Clarity.** One check for each backlog story whose text, criteria, linked
  nodes or decisions changed since its last check. A criterion that names a
  test becomes a test obligation, which the lane's test author owns and the
  verifier skips; the omni-stories skill writes test obligations under
  `Tests:` in the story's text instead. Once clarity has checked a story, its
  obligations are those it recorded, and nothing else: "The Test connection
  button shows Connected" is behaviour. A story clarity never checked, such
  as one implemented before it ran, is read for a test as the thing
  delivered: "A regression test covers ...", "Add a test ...", "Tests cover
  ...". The story versions that the old runner kept in `clarity.json` are not
  checks of this kind, since it recorded no test obligation. A minor gap takes
  its recommendation.
  The 3 blocking gaps of most impact go to the council, and the rest take
  their recommendation.
- **The council** answers a question before the human is asked: clarity's
  blocking gaps, and the questions of the implementer, the reviewer and the
  test author. Three members each read one thing: the spec docs and decisions,
  the code, or what the user would expect. When all three pick the same
  option, and a member quotes its source or all three call the gap minor, the
  answer becomes an agent decision and the story goes on. Otherwise a thread
  opens with each member's pick and evidence, and the story is blocked until
  the thread has a conclusion.
- **Deliver,** in the three stages below.
- **Report.** A check of the user journeys across the stories landed, then the
  report. The journeys check runs none of the checks, since an agent's Bash
  call stops after 10 minutes: it is told the result of the lane's last one.

The run does not wait for answers: the next run's Unblock takes them.

## The three stages and the lane

A story goes through the board's three working stages: implementing, review
and testing.

1. **Implementing,** at most `implementers` stories at once. The board is
   listed for the next story only once the last one's claim is made, since a
   claimed story's planned files keep the stories planned on them off the
   pickable list. The implementer works in the story's worktree,
   `<stories dir>/<key>`, on the branch
   `story/<key>`. A new worktree is cut from the tip of `deliver/<slug>`; one
   from an earlier attempt is kept, with its commits, and its story is picked
   first. From the second attempt on, the implementer runs on `implRetry`,
   and reads every earlier finding, by attempt, on the board. Implementers
   write no tests and never start the stack. A worktree an implementer left with another branch or a
   detached HEAD checked out goes back on `story/<key>` when it is clean, and
   the next implementer is told which commits to bring over with
   `git cherry-pick`; one with work in progress (uncommitted changes, a rebase
   in progress, an `index.lock`) stays as it is, and the next implementer is
   told what to do first.
2. **Review,** at most `reviewers` stories at once. A read-only reviewer gives a
   verdict on every criterion and lists every defect, and the script decides
   the review from those. Test obligations are not judged.
3. **Testing,** one story at a time, in the lane worktree `<stories dir>/_lane`.
   It stays from run to run, on a detached HEAD.
   - `lane-prepare` rebases `story/<key>` onto the tip of `deliver/<slug>` in
     the story's worktree, and checks the result out in the lane worktree.
   - The checks `fast`, `stack_up` and `stack` run there, in that order, as
     jobs. The run's first `stack_up` comes after `stack_reset`, when it is
     set, for what a run that was stopped left on the stack.
   - A verifier exercises every criterion but the test obligations, on the
     stack that `stack_up` started.
   - A test author writes the story's regression tests and commits them on the
     lane's detached HEAD. `lane-clean` throws away what it left uncommitted,
     so that its check, which runs next as a job, runs on exactly what lands,
     and again after the check, for what the check left, such as a report.
   - `lane-land` lands the story, moves it to done, and removes its worktree
     and `story/<key>`.

   The test author's trouble costs the verified story nothing. A check of its
   tests that comes out red gets the author once more, with the check's
   output. When the check is still red, or `lane-land` refuses the lane the
   author left (a branch checked out, files a process it started wrote), the
   tests are dropped: `lane-candidate` puts the lane back at the story as its
   checks and verifier found it, the story lands without them, and the
   report notes "landed without regression tests" with the reason. A defect
   its tests find is the story's, and uses an attempt.

What sends a story back:

- A failed review or verification, or a defect the test author's tests find,
  uses one of the story's 3 attempts since the human's latest answer. The
  story stays held where it failed until an implementer's slot is free, ahead
  of new stories; then one helper step, `rework`, moves it to the backlog and
  claims it again, and its next implementer goes on in its worktree. After the
  third, the story is blocked on a thread with every attempt's findings, which
  the helper writes from the board.
- A failed lane check runs once more on the tip without the story, after
  `stack_reset` when the check is `stack_up` or `stack` (see The checks). When
  it passes there, the failure is the story's, and uses an attempt. When it
  fails there too, the trouble is not the story's: the lane stops for the rest
  of the run, with one thread for the run, and the story goes back to reviewed
  with no attempt used. Implementers and reviewers finish what they hold. For
  a stack check, the thread names the likely cause: a story left state on the
  stack, which `stack_reset` (or `stack_up`) should reset.
- A rebase conflict sends the story back with no attempt used, to rebase and
  resolve in its worktree, twice per story in a run: from the third, a
  conflict uses an attempt, so that one resolved by a merge, which the lane's
  rebase meets again, cannot cycle. Uncommitted changes there, a rebase left
  in progress, an `index.lock`, or another branch checked out, use an
  attempt.
- An agent that returns nothing is asked again after 30 s and after 2 minutes,
  and then its story goes back to where it waited, with no attempt used. When
  agents return nothing for 3 stories in a row, the run takes no new story and
  opens one thread for the run.
- A story whose delivery stops on an error is released and blocked on a thread
  with the error. When the same error stops a second story, the run takes no
  new story and opens one thread for the run.

After a failure in the lane, the lane worktree goes back to the tip, and when
the stack holds the story's build, `stack_reset` (when set) and `stack_up` run
again there, so that the test stack is left on the build of `deliver/<slug>`,
without what the story left in it. A story that fails at `fast` never started
the stack, and nothing is rebuilt. When the lane's turn stops on an error, a
check still running is stopped first, so that it never runs beside the next
story's checks.

## Landing on deliver/<slug>

The slug is the feature's name in lowercase, with every character other than
a-z and 0-9 turned into `-`: the feature "Rename a project" lands on
`deliver/rename-a-project`. The branch is checked out nowhere. At a run's
start:

- when it is missing, it is created at the checkout's HEAD;
- when HEAD is ahead of it, it is fast-forwarded to HEAD;
- when it is ahead of HEAD, with the stories of an earlier run, it is kept;
- when the two have diverged, the run stops before any work and says to merge
  first.

`lane-land` moves the branch with a compare-and-swap, from the tip the story
was tested on to the tested commit, and checks that the branch then holds the
tested tree. A branch that moved in between is not swapped.

The report lists the stories landed, with their commits. To bring them into
main:

```bash
git merge --ff-only deliver/<slug>
```

When main has moved on since, `git merge deliver/<slug>` instead; the next run
fast-forwards the branch to that merge.

## The checks

A run's checks come from the `[checks]` table of the project's `.omni.toml`,
which the run's start reads with `omni checks --json`. `args.checks` sets
others on top, for one run. The workflow treats every check as optional: one
that is unset, or set to an empty command in `args.checks`, is skipped, and the
prompts leave it out.

| Check | Run by | What for |
| --- | --- | --- |
| `fast` | the lane, as a job | the project's own tests and checks |
| `stack_up` | the lane, as a job | builds and starts the test stack from the lane worktree, and builds it again after a failure |
| `stack` | the lane, as a job | the tests that need the stack |
| `stack_reset` | the lane, as a job | resets the test stack's state, such as its database, before the run's first `stack_up`, before a stack check runs again on `deliver/<slug>`, and before the stack is rebuilt after a failure |
| `seed` | the verifier | a credential on the test stack: it prints what to sign in with |
| `lint` | the verifier | a criterion about lint, which the verifier checks by running it, as it does `fast` and `stack` |

Every check the lane runs has `OMNI_STORY` (the story's key) and `OMNI_RUN`
(the run) in its environment, and the verifier runs the seed with them too,
so that a project can tell stories and runs apart: a seed that makes a user
of its own for each story, for one.

The omni CLI on main requires `fast`, `stack_up` and `stack`, and gives only
those three. The branch `omni-checks-optional` makes every check optional and
passes the others through. Until it is merged, `args.checks` is how a run gets
`stack_reset`, `seed` and `lint`.

Every check runs as a job: a detached supervisor runs the command in the lane
worktree, in a process group of its own and with no standard input, and logs
its output under `.omni/runs/<run>/jobs/`. A courier waits for it up to 8
minutes at a time. A job has 45 minutes: past that, its group gets SIGTERM,
then SIGKILL 10 s later, and the check counts as failed. Its finding quotes the
end of the log, which the helper reads, and which says that the job ran past
its limit. A workflow
stopped from outside leaves its jobs running, and the next start stops them;
finish stops those of its own run.

A stopped check can leave behind what it started outside its process group,
such as the container of a `docker compose run`, which runs in Docker's VM.
Cleaning that up is the project's job, in its `stack_up` or a preflight. This
repo's `stack_up` does not do it yet: a `stack` check stopped at its limit can
leave its test container running, which
`docker ps --filter label=com.docker.compose.project=omnistate-test` lists.

## The lock

`.omni/run.lock` keeps one run to a checkout. It names the run, the feature,
and by its pid the Claude Code session that runs the couriers, and every
helper call of the run refreshes its heartbeat. A start while that run is alive
is refused. The lock is stale once its session has ended, or once the run has
made no call for 2 hours, and the next start takes it over and says so; `force`
takes over a lock whose run the human knows is dead. A run stopped from
outside, by stopping the workflow in its session, keeps its lock: the session
lives on and the heartbeat is recent, so a start is refused until 2 hours
after the run's last call. Start the next run with `force: true`. The helper finds the session as the
nearest process above it that is not a shell, which has been checked on macOS
only.

`.omni/` also holds the clarity cache, `clarity.json`, and a folder for each
run, `.omni/runs/<run>/`: the payloads in flight, the record of applied steps,
the jobs with their logs, the timings and the report. A run's start adds
`.omni/` to `.git/info/exclude` when git does not ignore it already; this
repo's `.gitignore` does.

## The report and the timings

The helper records the start and the end of every step and every job in
`.omni/runs/<run>/timings.jsonl`. Every run that started ends with `finish`,
after an error too: it stops the run's jobs that still run, writes the report
to `.omni/runs/<run>/report.md`, posts it on the run's thread, and gives the
lock up. When the post fails, the file still holds the report.

The report has:

- the run's notes, such as why it stopped taking stories, and the stories
  that landed without regression tests, with the reason;
- the stories landed on `deliver/<slug>`, with their commits and the command
  that brings them in;
- the blocked stories, with their threads, from the board;
- every story's outcome;
- how many questions the council settled, and how many it sent to the human;
- the human checks still pending;
- the journeys check;
- the timings: for each story its total time, its time in each stage, its
  rebases, and each of its jobs by name. A time with (n) after it adds up n of
  them.

## Limits

- **The agent cap.** The runtime stops a workflow at 1000 agent calls,
  couriers included, and one story has taken 89, through three failed passes
  of the lane. A run takes a new story only while its calls leave room under
  900 for the stories in flight, at 90 each, and stops a story that reaches
  960 all the same, released with no attempt used. A long feature takes more
  than one run: the report says when a run stopped for the cap, and the next
  run carries on.
- **Small machines.** The runtime runs at most min(16, CPUs - 2) agents at
  once, couriers included. With 4 CPUs that is 2, and a courier waiting on a
  check holds one of them. The run still finishes, just slowly.

## The test stack

A run reads its board from the OmniState instance on this machine, and its
lane builds and tests each story on a second one. Those are two compose
projects of the same `docker-compose.yml`:

| Project | Address | Holds | Started by |
| --- | --- | --- | --- |
| `omnistate` | http://omnistate.localhost | the board the run claims stories from, and the human's data | `docker compose up` (never a run) |
| `omnistate-test` | http://omnistate-test.localhost | the build of the story in the lane, or of `deliver/<slug>`, and whatever the stack tests wrote | the lane's `stack_up` check |

Both route through the edge (`docker/edge/README.md`); without it they are
http://localhost:8420 and http://localhost:8421. Each project has volumes of its
own (`omnistate-test_postgres-data`, `omnistate-test_rustfs-data`,
`omnistate-test_test-target`, `omnistate-test_test-cargo-registry`), so nothing
a run builds, migrates or writes reaches the board.

This repo's `.omni.toml` sets three checks. `fast` runs lint, the WebAssembly
renderer's build, and the Rust, web and workflow tests. The stack checks are:

- `stack_up`: `OMNI_PORT=8421 docker compose -p omnistate-test up --build --wait --detach proxy`
- `stack`: `OMNI_PORT=8421 docker compose -p omnistate-test run --rm test`

They run in the lane worktree, so `stack_up` builds the images from the tree
in the lane. The same two stack steps are `make test-stack-up` and
`make test-stack` for running them by hand. The lane runs `fast` first and
`stack_up` only after it passed: the web image copies `web/public/render`,
which the fast command's `make render-wasm` rebuilds, so a stack built at the
same time served a stale renderer and the verifier checked the wrong build.

### A credential on the test stack

The test stack starts with no users, and the permission classifier refuses an
agent that writes into its database by hand. The verifier gets a credential
from the run's `seed` check, which it runs in the lane worktree. In this repo
that command is

```bash
make test-stack-seed NAME=us26
```

which inserts a workspace, a user `<NAME>@example.com` with a live session and
a membership, mints an API key through the API, and prints three lines:
`workspace=<id>`, `session=<token>` and `api_key=<secret>`. A human route takes
the cookie `omni_session=<token>`; an agent route, and the omni CLI with
`OMNI_SERVER=http://localhost:8421`, take `Authorization: Bearer <secret>`. The
rows live in the test database until `make test-stack-down` removes its
volumes, and a NAME used before fails, since a user's email is unique.

`.omni.toml` does not set it as the `seed` yet, so a run gets it, with a new
NAME each time, through its args, with the `stack_reset` the next section
explains:

```json
{"feature": "<name>", "checks": {"stack_reset": "make test-stack-down", "seed": "make test-stack-seed NAME=lane$(date +%s)", "lint": "make lint"}}
```

Without a seed, the verifier has no credential on the test stack. The seed
runs with `OMNI_STORY` and `OMNI_RUN` set, but a story can run it more than
once in a run, so the NAME above takes the time as well.

### Bringing the test stack down

```bash
make test-stack-down
```

which is `docker compose -p omnistate-test down --volumes`. The next story in
the lane (or `make test-stack-up`) creates it again from scratch; the first
build after that compiles the test binaries into a fresh `test-target` volume,
which takes a while.

The API applies its migrations on start, and refuses to start on a database
that holds a migration it does not carry, or one that changed after it was
applied. So a story the lane sends back after its `stack_up` leaves its
migrations in the test database, which `deliver/<slug>` never got, and from
then on `stack_up` fails for every tree without them. That is what
`stack_reset` is for: the lane runs it before a stack check runs again on the
base, and before it rebuilds the stack after a failure, so that both start
from an empty database and the failure stays the story's. It runs it before
the run's first `stack_up` too, since a run that was stopped can leave a
story's migrations behind, which the next run's first story would otherwise
pay an attempt for. With `make test-stack-down` as `stack_reset`, each reset
also drops the build caches, and the `stack_up` after it compiles the test
binaries again.

Without a `stack_reset`:

- the rebuild after the failure fails, and the next story's `stack_up` fails
  on the story and on the base, which stops the lane, as trouble that is not
  the story's;
- a story with a migration whose `stack` check fails stops the lane at once,
  since the rerun on the base meets its migration, and it does so again in
  every run after, with no attempt used.

The lane-stop thread names that likely cause, and
`docker compose -p omnistate-test logs api` shows it: `migration <n> was
previously applied but is missing in the resolved migrations`. Run
`make test-stack-down` before the next run.
