# The delivery stack

A delivery run is the `omni-deliver` workflow (`workflows/omni-deliver.js`),
which Claude Code runs with its Workflow tool. It takes a feature's stories
from the OmniState board, implements, reviews and tests them, and lands them on
the branch `deliver/<feature slug>`; each story's regression tests are written
once it has landed, and its changes are written back into the feature's
`Operations` and `Tables`. The run ends with that branch pushed and one pull
request, whose text is the report. Only the human starts it, never another
skill or workflow. Here a task is a story record on the board: the commands
(`omni story ...`), the keys and the helper's steps still say story. The
script holds the flow, but it can run no command itself: every git, omni, gh
and check command runs in its host helper, `workflows/deliver-step.mjs`,
which a cheap courier agent runs for each step. The helper's header describes
that protocol and every step; the script's `meta` sums up a run.

A courier call takes about ten seconds, of which the helper works a fraction
of one, so steps that always follow each other with nothing to decide in
between go in one call: the helper's `batch` step runs them in order, each as
a step of its own, recorded and timed as one, and stops at the first that
fails; a repeat of the batch carries on after those that were applied. A
story's claim comes with its context and its worktree, an implementer's
decisions with the story's files and its move to implemented, the claim for
the lane with the rebase and the context, and `lane-clean` with `lane-land`.
A clean story takes 9 courier calls from its claim to its landing, and its
tests 4 more, outside the lane's lock but for the last three; with a call for
every step, and the tests written in the lane, a story took 23.

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
  before the human's latest answer, and it reads the nodes the story links
  with `omni doc read <doc> --node <id>`, not the whole docs, which can be
  hundreds of KB. The clarity check reads the spec, architecture and db docs
  whole.
- An agent that needs a check's output reads the job's log by the path the
  prompt names. A finding about a check names the job, and the helper adds
  the end of its log.
- The helper writes the report's lists of the tasks that wait, failed or were
  not started, and the questions they wait on, from the board.

A courier copies the payload it writes for the helper the same way, so no
payload carries an agent's text either: no string in one passes about 500
characters, however long a reviewer's evidence or a council member's quote.
Every role agent writes its result, the object it returns, to
`.omni/runs/<run>/out/<label>.json` as well. A payload names the parts of that
file a step writes, such as a finding's evidence or a decision's choice, with
the fnv1a of those parts as the script got them back from the agent, and the
helper reads them from the file. The script keeps the file's path on the
result itself, so a runtime that hands it a copy changes nothing.

A file that is missing, or whose parts hash otherwise, is the agent's slip,
and it cannot change once the agent has finished: the helper refuses it for
good, naming the file, and the script sends the step again with that file's
parts carried another way. Parts of 500 characters or less in all go in the
payload, as text. Longer ones come from a file that the same role writes once
more from the result. When that file is wrong too, they go in the payload cut
to 500 characters, with a note that the agent's full result could not be
carried. This holds for each council member's file on its own, and for the
report's journeys. A missing file never blocks a story, and an error about one
never counts toward the stop on a repeated error.

## Starting a run

Once per machine, `make install-agent-tools` links the workflow at
`~/.claude/workflows/omni-deliver.js`, its agent types at
`~/.claude/agents/omni-*.md`, and its helper at
`~/.claude/omni/deliver-step.mjs`, the path the workflow runs it from. The
links point into this checkout.

The agent types are not required: the runtime refuses an agent type that is
not installed, and the workflow then runs that role without it, as the default
agent with every tool, for the rest of the run. It tries each type once per
run, and the log and the report's notes say once per type: "agent type X is
not installed (make install-agent-tools); running role Y without it". Any
other error from the runtime is rethrown unchanged.

### The agent types

Every tool definition an agent has is in its starting context: a haiku
`claude -p` with every default tool starts at 44,200 tokens, and agents in
real runs started at about 65k. So each role runs as an agent type of its own,
in `workflows/agents/`, whose `tools:` lists only what the role uses, and whose
body says no more than that the prompt defines the task:

| Role | Agent type | Tools |
| --- | --- | --- |
| `courier` | `omni-courier` | Bash, Write, Read |
| `clarity`, `council`, `review`, `docs` | `omni-clarity`, `omni-council`, `omni-review`, `omni-docs` | Read, Grep, Glob, Bash, Write |
| `impl`, `implRetry` | `omni-impl` | Read, Edit, Write, Bash, Grep, Glob, Monitor, ToolSearch |
| `tests` | `omni-tests` | Read, Edit, Write, Bash, Grep, Glob, Monitor, ToolSearch |
| `verify`, `final` | `omni-verify`, `omni-final` | Read, Grep, Glob, Bash, Write, Monitor, ToolSearch, `mcp__Claude_Browser` |

Write is for the result file every role writes, and Read for the file the
Write tool refuses because it exists. Monitor goes with ToolSearch: with
ToolSearch, Monitor is a deferred tool and costs a line, without it its whole
definition. The test author reads the writing-tests skill by its path, so it
has no Skill tool, whose list of every skill is about 11k tokens.

A `tools:` entry names an MCP tool as `mcp__<server>__<tool>`, and every tool of
a server as `mcp__<server>`: `mcp__Claude_Browser` is the desktop app's
browser, preview included. A name the machine lacks is left out without an
error, so the verifier on a CLI without that browser runs without one.

`make measure-agents` prints each type's starting context: a haiku `claude -p`
with the type's tools and body, asked for one word. It spends real tokens, so
no test runs it. A tool the CLI lacks is not counted.

A run starts from a Claude Code session in the project's main checkout, or,
for a new project, in its folder (see A new project):

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
| `models` | see below | the model of a role: `clarity`, `council`, `impl`, `implRetry`, `review`, `verify`, `tests`, `final`, `docs` or `courier` |
| `agentTypes` | `omni-<role>` | the agent type of a role (see The agent types); `implRetry` is set apart from `impl` |
| `helper` | `~/.claude/omni/deliver-step.mjs` | another path to the helper |

Every role runs on opus, except `impl`, a story's first attempt, and `docs` on
sonnet, and `courier` on haiku.

The run pushes once, at its end: the feature's branch, for its pull request
(see The pull request). It merges nothing, and never moves the checkout's
branch. Outside a setup run it never changes the checkout's files either: it
reads the checkout's HEAD at its start, and keeps its own state in `.omni/`,
which git ignores.

## What a run does

- **Start.** The helper takes the lock, sets `deliver/<slug>`, reads the checks
  and opens the run's thread, `Run <run id>`. It releases the stories a dead
  run still holds, stops the jobs of earlier runs that still run, and cleans
  the lane worktree with `reset --hard` and `clean -fd`, never `-x`, so that
  ignored build caches stay warm. In a setup run it does nothing with git
  (see A new project).
- **Unblock.** A blocked task whose questions are all answered gets each
  answer as a decision, its open findings resolved, and is unblocked, with a
  new round of tries (see Questions). Then each task parked after 3 tries is
  taken up again (see Parking).
- **Setup.** In a setup run only: the task `Set up the project`, first and
  alone, straight in the folder (see A new project).
- **Clarity.** One check for each backlog story whose text, criteria, linked
  nodes or decisions changed since its last check. A criterion that names a
  test becomes a test obligation, which the story's test author owns and the
  verifier skips. The check names it by its number, and the helper records
  the decision with the criterion's text from the board; a number the story
  has no criterion for fails the check, and the story waits for the next
  run; the omni-stories skill writes test obligations under
  `Tests:` in the story's text instead. Once clarity has checked a story, its
  obligations are those it recorded, and nothing else: "The Test connection
  button shows Connected" is behaviour. A story clarity never checked, such
  as one implemented before it ran, is read for a test as the thing
  delivered: "A regression test covers ...", "Add a test ...", "Tests cover
  ...". The story versions that the old runner kept in `clarity.json` are not
  checks of this kind, since it recorded no test obligation. A criterion that
  asks that `Operations` and `Tables` say what the landed code does is
  recorded the same way, as the write-back (see The write-back). A minor gap
  takes its recommendation.
  The 3 blocking gaps of most impact go to the council, and the rest take
  their recommendation.
  The checks run beside the delivery: the run starts them all and goes on to
  Deliver at once. The stories are still taken in the board's order, which is
  the human's priority and decides which of two stories planned on the same
  files goes first: the run waits for the check of the next story in that
  order, and takes no later story ahead of it. So the first story is
  implemented while the checks of those after it still run. The run waits for
  every check before its report.
- **The council** answers a question before the human is asked: clarity's
  blocking gaps, and the questions of the implementer, the reviewer and the
  test author. Three members each read one thing: the spec docs and decisions,
  the code, or what the user would expect. When all three pick the same
  option, and a member quotes its source or all three call the gap minor, the
  answer becomes an agent decision and the story goes on. So it does when two
  of the three pick the same option and each of the two quotes its source:
  the decision then says it was 2 of 3 and names the third member's pick, so
  the human can see the split. Otherwise, with no majority or a majority
  without a quote from each of its two, the question goes to the human on a
  node of the feature's behaviour tree, with each member's pick and evidence
  and a recommendation, and that task waits for the answer (see Questions).
- **Deliver,** in the three stages below.
- **Tests.** Each story's regression tests, written once it has landed, by
  one author at a time, beside the delivery of the stories after it (see
  below).
- **Report.** Once every clarity check, every story's tests and every
  write-back have ended, a check of the user journeys across the stories
  landed, then `finish`: the report, the push and the pull request (see The
  pull request). The journeys check runs none of the checks, since an agent's
  Bash call stops after 10 minutes: it is told the result of the lane's last
  one.

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
   - A verifier exercises every criterion but the test obligations and the
     write-back, on the stack that `stack_up` started, and compares the built
     screen with each design board the story links (see The design check).
   - `lane-clean` throws away what the verifier left uncommitted, and
     `lane-land` lands the story and moves it to done. Its worktree and
     `story/<key>` stay, for its test author. A lane that `lane-land` refuses
     for its state, such as a branch an agent checked out there, goes back to
     the story as it was checked (`lane-candidate`), and the story lands from
     that. Once it has landed, a docs agent writes its changes into
     `Operations` and `Tables` (see The write-back).

A story's **regression tests** are written once it has landed, outside the
lane's lock: in the lane, their author took 2 to 25 minutes a story, with the
lane and the stack held, and found no defect in 8 stories of 8. The story is
done by then, so the stories that waited on it are picked while its tests are
written.

- One author works at a time, for the whole run, and holds neither the lane
  nor an implementer's or a reviewer's slot. `tests-prepare` brings the
  story's worktree to the tip of `deliver/<slug>`, on `story/<key>`, so the
  author starts from every test that landed before it. Its oracle comes from
  the story's text, criteria, decisions and linked doc nodes: it ignores the
  story's findings. It never starts or uses the stack, which may hold another
  story's build by then: a risk only a stack test would catch goes in
  `notAdmitted`.
- Its commits take a short turn in the lane. `lane-prepare` throws away what
  the author left uncommitted in its worktree, rebases `story/<key>` onto the
  tip, and checks it out in the lane. The author's check runs there as a job,
  with no `fast`, `stack_up` or `stack` check, and the stack is left alone.
  `lane-clean` throws away what the check left, such as a report, and
  `lane-land` lands the tests by the same compare-and-swap, adds their files
  to the story's, and removes the worktree and `story/<key>`. The lane is at
  the tip afterwards.
- A turn that fails, on a rebase conflict, a red check or a refused landing,
  puts the lane back at the tip, and the author is asked once more, in its
  worktree, with the reason and the path of the check's log. A second failure
  drops the tests.
- Dropped tests cost the story nothing: its worktree and branch go, and the
  report notes "landed without regression tests" with the reason. Besides a
  second failure, the tests are dropped when the author finds a defect, asks
  a question the council does not settle, is blocked, or returns nothing
  after its retries, when the lane has stopped for the run, and at the hard
  cap.
- A defect the tests find comes after the landing: it uses no attempt, the
  story is not sent back, and no thread opens. The report's note names each
  failing test, what it observed and what the story expects, for the human
  to decide.
- A question of the test author that the council does not settle blocks
  nothing, since the story is done: it is asked on a node of the behaviour
  tree like any other, without blocking the story, and in a feature without
  a tree its thread names the story in its title and is not linked from it.
  Either way the tests are dropped.
- A run stopped from outside while tests were pending leaves the worktree of
  a story that is done. The next start removes it and says so: those tests
  are not written.

What sends a story back:

- A failed review or verification, a criterion not met or a difference from
  a design board, uses one of the story's 3 tries in its round. The story
  stays held where it failed until an implementer's slot is free, ahead of
  new stories; then one helper step, `rework`, moves it to the backlog and
  claims it again, and its next implementer goes on in its worktree. After
  the third, the story is parked (see Parking).
- A failed lane check runs once more on the tip without the story, after
  `stack_reset` when the check is `stack_up` or `stack` (see The checks). When
  it passes there, the failure is the story's, and uses an attempt. When it
  fails there too, the trouble is not the story's: the lane stops for the rest
  of the run, which the report's notes say, and the story goes back to
  reviewed with no attempt used. Implementers and reviewers finish what they
  hold. For a stack check, the note names the likely cause: a story left
  state on the stack, which `stack_reset` (or `stack_up`) should reset.
- A rebase conflict sends the story back with no attempt used, to rebase and
  resolve in its worktree, twice per story in a run: from the third, a
  conflict uses an attempt, so that one resolved by a merge, which the lane's
  rebase meets again, cannot cycle. Uncommitted changes there, a rebase left
  in progress, an `index.lock`, or another branch checked out, use an
  attempt.
- An agent that returns nothing is asked again after 30 s and after 2 minutes,
  and then its story goes back to where it waited, with no attempt used. When
  agents return nothing for 3 stories in a row, the run takes no new story,
  which the report's notes say.
- A story whose delivery stops on an error is released, and the report lists
  it under Failed with the error. When the same error stops a second story,
  the run takes no new story, which the notes say.

None of these opens a thread: the run's trouble is a line in its report.

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
- when the two have diverged and `gh` lists a merged pull request of the
  branch whose head was its tip, it starts again at HEAD;
- when the two have diverged otherwise, the run stops before any work and
  says to merge first.

`lane-land` moves the branch with a compare-and-swap, from the tip the story
was tested on to the tested commit, and checks that the branch then holds the
tested tree. A branch that moved in between is not swapped.

The report lists the stories landed, with their commits. They reach the
checkout's branch through the feature's pull request, which the human merges
(see The pull request); the run merges nothing.

## A new project

A feature of a project with no code yet has a task titled
`Set up the project`, which the omni-stories skill writes. While that task is
not done or cancelled, a run on the feature is a setup run, whether the folder
has a repository or not:

- `start` takes the lock, opens the Run thread and probes the stories folder,
  and does nothing with git: `branch`, `tip` and `lane` are null and the
  checks empty. The root is the top folder of the checkout the current folder
  is in, else the current folder. A setup run started inside another checkout
  works in the folder `omni link` matches for the project, and is refused,
  naming that folder, when it is started anywhere else.
- The setup task runs first and alone, straight in the folder: no worktree,
  no branch of its own, no lane, no reviewer, no test author, no clarity
  check and no docs agent, and no other task is claimed before it has
  landed. One implementer runs `git init` unless the folder is the top of a
  repository already, sets up the stack the Project spec names, writes
  `.omni.toml` with a `fast` check, adds `.omni/` to `.gitignore`, commits,
  and last writes the project-level `Setup` doc (pillar `architecture`): the
  folders, the libraries, and the commands to start the app and to run the
  tests.
- `setup-check` then wants a repository with a commit, nothing uncommitted
  outside `.omni/`, and a `fast` command; `fast` runs as a job in the folder,
  and one verifier exercises the task's criteria there. Each failing step is
  a finding and uses a try; after the third the task is parked.
- Once checked, `setup-land` records the repository's files on the task,
  ticks its criteria and makes it done; the report lists it under Built with
  the commits up to its head. `repo-open` does what `start` does with git in
  any other run: `deliver/<slug>` at HEAD, the checks and the lane worktree.
  The other tasks then run the normal way, in the same run.
- A setup task that does not land, or is blocked on a question, ends the run
  with nothing else built, which the report notes.

A folder without a repository whose feature has no such task is refused,
with the reason and a pointer to split the feature with omni-stories first.

Every later implementer and verifier is told to read the `Setup` doc, whose
ID `start` and `repo-open` give. A verifier in a project with no stack
command starts the app itself, with the start command the `Setup` doc gives.

## Questions

The council sits first, as above. A question it does not settle goes to the
human, and only its own task waits:

- In a feature with a behaviour tree, which `start` finds with
  `omni behaviour list`, the helper's `question-ask` runs
  `omni behaviour ask <node> "<question>" --recommend "<answer>"` on the node
  the asking agent names, else the task's first Covers node, else the root
  `.`, whichever the tree has first. The question carries its options and who
  asked. A decision on the task records the question's ID, as
  `Asked on the behaviour tree: question <id> on node <node>. The task waits for the answer.`,
  and the task is blocked; the other tasks go on.
- The report lists the task under Waiting on a question, with the node and
  the recommendation, and says to answer on the feature's Behaviour screen.
- The next run's `unblock-pass` reads each such question with
  `omni behaviour question <id> --json`. When every question of a blocked
  task is answered, and every thread linked from it has a conclusion, each
  answer becomes a decision of the task (by `human`, or `agent` when an agent
  answered it), its open findings are resolved, which starts a new round of
  3 tries, and it is unblocked. A question that cannot be read keeps its task
  blocked and says so in the log.
- A feature without a behaviour tree has no node to ask on: there the
  question opens a thread linked from the task, which blocks it, and the
  thread's conclusion is the answer.

A clarity check sends at most 3 blocking questions; an implementer, a
reviewer or a test author gets at most 3 questions settled by the council in
one try, and the next goes to the human directly.

## The design check

A task points at a design board with a link whose doc is a design doc
(`omni doc list` gives `kind: "design"`) and whose node is the board's ID, as
omni-stories saves it. `context` gives those links as `boards`, at most 20.

- Before the first implementer or verifier of a task with boards, the helper's
  `design-export` runs `omni export` once per run, into
  `.omni/runs/<run>/design`. Its `manifest.json` gives each board's tags file,
  with the board's PNG beside it. An export that fails stops no task: the
  report notes it, and the agents read each board's tags with
  `omni design read` instead.
- The implementer is told to build to the boards.
- The verifier opens the built screen, compares it with each board's image and
  tags, and returns for each board whether it compared it, why not, and its
  differences. A difference is one of: a part missing, a part extra, another
  order or place, other text, a missing state, a colour that is not the
  board's, a type style that is not the board's. Size and position are not
  compared.
- Every difference is a finding, `Differs from the board ...`, and sends the
  task back, using the same try as the criteria not met.
- A board the verifier did not compare, for one because it found no way to
  start the app, does not stop the landing: the report lists it under Not
  compared with its board, with the reason.

A task without a board is checked on its criteria only.

## Parking

A task gets 3 tries per round. A round starts on its first run, after an
answer to its question, and when a later run takes it up after parking.

- After the third failed try in a round, the helper's `park` leaves the task
  in the backlog, not blocked, with its findings open, and removes its
  worktree and `story/<key>`. No thread opens.
- The report lists it under Failed, `parked after 3 tries`, with the last
  try's findings. A task whose `blocked_by` names it is not pickable on the
  board, and the report lists it under Waiting for another task.
- The next run's `retry-pass`, after the unblock pass, finds every task in
  the backlog, not blocked, with an open finding marked `[attempt 3]`, and
  resolves its open findings: its tries count from 0 again, with fresh
  agents.

No failure of a task or of a run opens a thread. The Run thread, with the
report, stays.

## The pull request

`finish` is the only step that pushes. In order, it stops the run's jobs that
still run, writes the report, pushes `deliver/<slug>` to the remote of the base
branch (its `branch.<base>.remote`, else `origin`), and opens one pull request
into the base with `gh pr create`, titled with the feature's name, or updates
the open one with `gh pr edit`. The base is the branch the checkout had
checked out. The pull request's text is the report; past 60,000 characters it
loses its Timings, and then is cut with a line that points at the Run thread.
Then the report, with the pull request's address as its second line, goes on
the Run thread and into `.omni/runs/<run>/report.md`, and the lock is given
up. The helper never runs `gh pr merge` and never moves the base branch.

Where no pull request can be opened, nothing is pushed and the report's second
line says why and names the branch:
`No pull request: <why>. The work is on the branch deliver/<slug>.` The
cases, the first that holds:

- the feature has no branch yet, as after a setup run whose task did not land;
- the checkout has no branch checked out;
- the checkout has no remote;
- the base branch is not on the remote;
- `gh` is not installed, or cannot list pull requests there;
- `deliver/<slug>` holds nothing the base lacks.

A push that fails, or runs past 5 minutes, gives git's reason the same way;
a `gh` that fails after the push gives its own.

Once the human has merged the pull request, the next run finds
`deliver/<slug>` diverged from HEAD with a merged pull request at its tip, and
starts the branch again at HEAD. Its push then moves the remote branch only
while it is still at the merged tip, or expects none there when the forge
deleted it.

## The write-back

A feature whose how has the docs `Operations` (pillar `architecture`) or
`Tables` (pillar `db`) gets the changes of each landed task written into them.
`start` gives their IDs.

- The criterion that asks for the write-back is recorded by the clarity check
  as a write-back obligation, as test obligations are, so neither the reviewer
  nor the verifier judges it before landing. The implementer leaves those docs
  alone and names in its decisions each operation and table it built or
  changed.
- Once `lane-land` has answered landed, a docs agent (role `docs`, agent type
  `omni-docs`) reads the landed diff and edits the two docs by the rules of
  `agent-skills/omni-how/write-back.md`, which its prompt carries in full. It
  runs outside the lane's lock, one write-back at a time, and returns
  written, nothing or blocked.
- The task stays done whatever the agent returns. Blocked, nothing returned
  after its retries, an error or the hard cap add a note:
  `<key> landed without its write-back`.
- The run waits for every write-back before its report. The `Map` doc is not
  touched, and the setup task gets no write-back.

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
its output under `.omni/runs/<run>/jobs/`. The courier that starts a job
waits for it too, up to 8 minutes, so a check that ends within that takes one
call; while the job still runs, a courier waits again, up to 8 minutes at a
time. A job has 45 minutes: past that, its group gets SIGTERM,
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
run, `.omni/runs/<run>/`: the payloads in flight, the agents' results, the
record of applied steps, the jobs with their logs, the timings and the report.
A run's start adds `.omni/` to `.git/info/exclude` when git does not ignore it
already; this repo's `.gitignore` does.

## The report and the timings

The helper records the start and the end of every step and every job in
`.omni/runs/<run>/timings.jsonl`. Every run that started ends with `finish`,
after an error too: it stops the run's jobs that still run, writes the report,
pushes and opens or updates the pull request (see The pull request), writes
the report to `.omni/runs/<run>/report.md`, posts it on the run's thread, and
gives the lock up. When the post fails, the file still holds the report.

The report is the pull request's text. Each task is in one of Built, Waiting
on a question, Failed, Waiting for another task and Not started at most, the
first that holds; a board that cannot be read leaves the lists that come
from it as unknown, which the report says. It has, in order:

- its title, then the pull request's address, or why there is none, naming
  the branch;
- the run's notes, such as why it stopped taking stories, why the lane
  stopped, the stories that landed without regression tests or without their
  write-back, with the reason, and a defect a story's tests found after it
  landed;
- Built: the tasks landed on `deliver/<slug>`, with their commits, each
  task's tests on a line of their own;
- Waiting on a question: each open question, with its node and its
  recommendation, or the open thread, from the board;
- Failed: the tasks parked after 3 tries, with the last try's findings, and
  those stopped on an error, with the error;
- Waiting for another task: the backlog tasks held through `blocked_by` by a
  task that waits or failed, with the keys they wait for;
- Not started: the rest of the backlog;
- Not compared with its board: the boards a verifier did not compare, with
  the reason;
- the human checks still pending;
- how many questions the council settled, how many of those by a majority of
  2, and how many it sent to the human;
- the journeys check;
- the timings: for each story its total time, its time in each stage, the
  time the writing of its tests took after it landed (test writing), its
  rebases, and each of its jobs by name. A time with (n) after it adds up n of
  them.

## Limits

- **The agent cap.** The runtime stops a workflow at 1000 agent calls,
  couriers included. A story that fails the lane three times makes about 50;
  one took 89 when every step had a call of its own, which is where the 90
  set aside for a story comes from. A run takes a new story only while its
  calls leave room under 900 for the stories in flight, at 90 each, for the
  clarity checks in flight, at 20 each, for the landed stories' tests
  still to be written or landed, at 30 each (a chain of them makes 5 calls
  when all goes well, and 23 with two questions settled and a failed turn in
  the lane), and for the write-backs still to run, at 5 each; a story that
  only the checks, the tests or the write-backs leave no room for waits until
  one of them has ended. It stops
  a story that reaches 960 all the same, released with no attempt used. A
  long feature takes more than one run: the report says when a run stopped
  for the cap, and the next run carries on.
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

The report's note on the lane stop names that likely cause, and
`docker compose -p omnistate-test logs api` shows it: `migration <n> was
previously applied but is missing in the resolved migrations`. Run
`make test-stack-down` before the next run.

## The next stack

The build of the projects, parts and tracks branch runs as a third compose
project, `omnistate-next`, with an empty database, so that the human's
`omnistate` keeps its data and keeps working. It is http://omnistate-next.localhost
through the edge (http://localhost:8422 without it). Its router and service
names in the edge follow the project name, so `http://omnistate.localhost` still
serves `omnistate` only. Its images are `omnistate-next-*` and its volumes
`omnistate-next_postgres-data` and `omnistate-next_rustfs-data`.

```bash
make next-up     # builds from the current tree, starts the edge if needed, seeds nothing
make next-down   # removes the containers and network, keeps the volumes
```

To sign in, create the first member with the API's own setup command, which
works on an empty database and gives the first member the highest role (admin):

```bash
docker compose -p omnistate-next exec api omni-api setup --email <you>@example.com --workspace <name> --password-stdin
```

It reads the password from stdin (at least 8 characters; end the line with
Enter). Then sign in at http://omnistate-next.localhost with that email and
password. The database has no projects until you create them. To start over,
`docker compose -p omnistate-next down --volumes`.
