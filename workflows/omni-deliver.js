export const meta = {
  name: 'omni-deliver',
  description: 'Delivers a feature\'s stories from the OmniState board: clarity checks with a council in front of every question for the human, implement and review in story worktrees, at most `implementers` stories implementing and `reviewers` in review at once, then a lane that tests one story at a time in a worktree of its own and lands it on deliver/<feature slug>, and a report on the Run thread. Every git and omni command runs in the host helper, reached through courier agents',
  whenToUse: 'Run it with the Workflow tool, as the workflow omni-deliver with args {feature: "<name>"} (optional: implementers (3), reviewers (3), storyLimit, force, checks, models, agentTypes, helper), from a session in the main worktree of a project whose folder is linked to its OmniState project (omni link, or OMNI_PROJECT). That session needs the stories folder beside the checkout, <checkout>-stories, among its working directories, and the host helper at ~/.claude/omni/deliver-step.mjs, as make install-agent-tools links it; args.helper names another path. One run per checkout: the helper\'s .omni/run.lock refuses a second live run, and force takes over a lock whose run the human knows is dead. args.checks ({fast, stack_up, stack, stack_reset, seed, lint}) sets checks on top of omni checks; every check command runs with OMNI_STORY and OMNI_RUN in its environment. args.models overrides the model of a role (clarity, council, impl, implRetry, review, verify, tests, final, courier), and args.agentTypes sets the agent type of a role, which has none by default. Stories land on deliver/<feature slug>, never on the checkout\'s branch: the report says to bring them in with git merge --ff-only deliver/<feature slug>. A run takes a new story only while its agent calls leave room under 900 for the stories in flight, at 90 each, and stops a story at 960, released with no attempt used; the next run carries on. An agent that returns nothing is asked again after 30 s and after 2 minutes, with no attempt used; agents that return nothing for 3 stories in a row stop new claims, with one thread for the run. A story whose delivery stops on an error is released and blocked on a thread with the error; when the same error stops a second story, the run takes no new story and opens one thread for the run, and later failures are only released. A failed check that fails again on deliver/<slug> without the story, after stack_reset for a stack check, stops the lane for the rest of the run, with one thread for the run, and the story goes back to reviewed with no attempt used. Tests whose author cannot get them to land are dropped, and the verified story lands without them, which the report notes. The run ends with the journeys check of the stories it landed and finish, which posts the report and gives the lock up, after an error too; it does not wait for answers, which the next run\'s unblock pass takes. Never pushes.',
  phases: [
    { title: 'Start', detail: 'the helper takes the lock, reconciles claims and the lane worktree, sets deliver/<slug> and reads the checks' },
    { title: 'Unblock', detail: 'a story whose threads all have a conclusion gets each one as a human decision and is unblocked, and its attempts count from there' },
    { title: 'Clarity', detail: 'one check per backlog story whose text, criteria, linked nodes or decisions changed: a criterion that names a test becomes a test obligation, and once a story is checked its obligations come from these decisions alone, a minor gap takes its recommendation, the 3 blocking gaps of most impact go to the council and the rest take their recommendation, deferred by the cap; what the council cannot settle becomes a thread that blocks the story' },
    { title: 'Deliver', detail: 'at most `implementers` stories implementing at once, each slot free again once its story is implemented: claim, worktree, implementer (on implRetry from attempt 2, with every finding by attempt), files; then at most `reviewers` in review, with a verdict per criterion and every defect, from which the script decides, never judging a test obligation; a failed review, or a failure in the lane, sends the story back for the next attempt, held where it failed until an implementer slot is free, ahead of new claims, then moved to the backlog and claimed in one step, and 3 failed attempts open a thread; a worktree an implementer left on another branch goes back on story/<key> when it is clean, and the next implementer is told which commits to bring over; a question goes to the council first' },
    { title: 'Lane', detail: 'one story at a time in the lane worktree <stories dir>/_lane: lane-prepare rebases story/<key> onto deliver/<slug> and checks it out, detached (a conflict or a missing worktree sends the story back with no attempt used, but from the third conflict of a story in a run; uncommitted changes, a rebase left in progress, an index.lock or another branch use one; trouble that is not the story\'s stops the lane); fast, stack_up and stack run there as jobs, and a failed one runs again on the base, after stack_reset for a stack check, where failing too stops the lane and passing uses an attempt; a verifier runs every criterion but the test obligations, a test author writes the regression tests and its questions go to the council, lane-clean throws away what it left uncommitted, its check runs as a job, lane-clean throws away what the check left, and lane-land moves deliver/<slug> by a compare-and-swap; a red check gets the author once more, with its output, and tests still red, or that lane-land refuses for the lane they left, are dropped, and the story lands without them; after a failure the lane goes back to the tip, and the stack, when it holds the story\'s build, is reset with stack_reset and rebuilt there' },
    { title: 'Report', detail: 'the journeys check of the stories landed, then finish posts the report on the Run thread, with the outcomes, the council\'s counts, the blocked stories with their threads, the notes and the timings, and gives the lock up' },
  ],
}

// omni-deliver as a native workflow (tasks/omni-deliver-native-design.md).
// The script holds the flow: which story goes where, attempts and retries,
// the council, the lane's lock and the report.
// It has no files, no processes and no clock, so every git, omni and check
// command runs in the host helper, workflows/deliver-step.mjs, which a
// courier agent runs for each step. No agent writes to the board itself.

// ---- args ------------------------------------------------------------------
const A = args && typeof args === 'object' ? args : {}
if (typeof A.feature !== 'string' || !A.feature.trim()) throw new Error('Pass the feature: args {feature: "<name>"}')
const feature = A.feature
const implementers = Number.isInteger(A.implementers) && A.implementers > 0 ? A.implementers : 3
const reviewers = Number.isInteger(A.reviewers) && A.reviewers > 0 ? A.reviewers : 3
// The distinct stories a run takes at most, for a first run on a feature.
const storyLimit = Number.isInteger(A.storyLimit) && A.storyLimit > 0 ? A.storyLimit : Infinity
// Where make install-agent-tools links the helper. The script cannot read
// HOME; the courier's shell expands the ~.
const HELPER = typeof A.helper === 'string' && A.helper ? A.helper : '~/.claude/omni/deliver-step.mjs'
// Opus judges and takes over rework, Sonnet builds the first attempt, Haiku
// carries the steps.
const MODELS = { clarity: 'opus', council: 'opus', impl: 'sonnet', implRetry: 'opus', review: 'opus', verify: 'opus', tests: 'opus', final: 'opus', courier: 'haiku', ...(A.models || {}) }
// No custom agent type by default: another machine may not have them.
const AGENT_TYPES = { ...(A.agentTypes || {}) }

// Attempts a story gets since the human's latest answer.
const MAX_ATTEMPTS = 3
// Blocking questions one clarity check asks; the rest take their
// recommendation, deferred by the cap.
const CLARITY_QUESTIONS = 3
// An implementer or reviewer runs again after each question the council
// settles. Past this many in one attempt, the next question goes to the
// human, so that a stream of settled questions cannot run on.
const QUESTIONS_PER_ATTEMPT = 3
// The runtime stops a workflow at 1000 agent calls, couriers included, and
// one story has used 89, through three failed passes of the lane. A story is
// claimed, and a clarity check started, only while the calls made and those
// set aside for the work in flight leave room under AGENT_CAP; the next run
// carries on. Work that reaches HARD_CAP all the same stops there: its story
// is released with no attempt used.
const AGENT_CAP = 900
const HARD_CAP = 960
const STORY_CALLS = 90
const CHECK_CALLS = 20
const COUNCILS = 2
// Rebase conflicts that send a story back with no attempt used, per story in
// a run. One the implementer resolves by a merge, which the lane's rebase
// meets again, would otherwise cycle with no end.
const CONFLICTS = 2
const COURIER_RETRIES = 3
// A review that leaves a criterion without a verdict is asked again this
// many times.
const REVIEW_RETRIES = 2
// The seconds one courier waits for a job, within its Bash call.
const JOB_WAIT = 480
// An agent that returns nothing is asked again after each of these waits, in
// seconds, with no attempt used. Agents that return nothing for this many
// stories in a row stop the run taking stories.
const AGAIN_AFTER = [30, 120]
const EMPTY_STORIES = 3

let agentCalls = 0
// Set once the run stops taking new work for the cap.
let capped = false
const room = (calls, inFlight) => agentCalls + calls * (inFlight + 1) <= AGENT_CAP
// past: the cleanup of work stopped at HARD_CAP, which the cap lets through.
const ask = (role, prompt, opts, model = MODELS[role], past = false) => {
  if (agentCalls >= HARD_CAP && !past) throw Object.assign(new Error(`the run has made ${agentCalls} agent calls, at its hard cap of ${HARD_CAP}`), { atCap: true })
  agentCalls++
  return agent(prompt, { ...opts, model, ...(AGENT_TYPES[role] ? { agentType: AGENT_TYPES[role] } : {}) })
}

// At most n holders at once; the others wait their turn, first come first
// served, after those that asked to go ahead.
const semaphore = (n) => {
  let free = n
  const ahead = []
  const queue = []
  const acquire = (first = false) => {
    if (free > 0) {
      free--
      return Promise.resolve()
    }
    return new Promise((resolve) => (first ? ahead : queue).push(resolve))
  }
  const release = () => {
    const next = ahead.shift() || queue.shift()
    if (next) next()
    else free++
  }
  const run = async (task) => {
    await acquire()
    try { return await task() } finally { release() }
  }
  return { acquire, release, run }
}

// ---- the courier -----------------------------------------------------------
// 32-bit FNV-1a over the string's UTF-16 code units, as 8 lowercase hex
// digits. The helper carries its own copy; a test checks that the two agree.
function fnv1a(text) {
  let hash = 2166136261
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return (hash >>> 0).toString(16).padStart(8, '0')
}

const quote = (s) => `'${String(s).replace(/'/g, "'\\''")}'`
// A path as a command carries it: quoted when the shell would read anything
// in it, with a leading ~/ left outside the quotes so that it still expands.
const shellPath = (p) => (/^~?[\w@%+=:,./-]+$/.test(p) ? p : p.startsWith('~/') ? `~/${quote(p.slice(2))}` : quote(p))

const COURIER_SCHEMA = {
  type: 'object',
  required: ['exitCode', 'output'],
  properties: {
    exitCode: { type: 'integer', description: 'The exit code of the command' },
    output: { type: 'string', description: 'What the command printed on standard output, exactly as printed' },
  },
}
// The payload goes through a file the Write tool creates, so that it never
// meets shell quoting or command hooks, and the Bash command stays short. A
// wait step can take 8 minutes, past the Bash tool's default of 2.
const courierPrompt = (command, file, payload) => `You are a courier: you carry one step of a workflow to its host helper. Do exactly what follows, and nothing else.
${file ? `
1. Write the payload. Use the Write tool to create this file:
${file}
Its whole content is the one line between the two marker lines below, copied character for character: change, add or drop nothing, not a space, a quote or a backslash. If the Write tool refuses because the file exists, Read the file, then Write it.
<<<PAYLOAD
${payload}
PAYLOAD>>>
` : ''}
${file ? '2' : '1'}. Run the helper. Use the Bash tool, with its timeout set to 600000, to run this command exactly as written, once:
${command}

Return exitCode, the exit code of the command, and output, what it printed on standard output, exactly as printed. Run nothing else, and fix or retry nothing: the workflow checks the output.`

// What a courier brought back: the helper's body once the envelope's hash
// matches its body and the helper received what the script sent, else what
// went wrong. The answer is the last line that starts as the helper's does,
// so that a warning the shell or node prints around it changes nothing.
const unpack = (r, expected) => {
  if (!r) return { problem: 'the courier returned nothing' }
  if (r.exitCode !== 0) return { problem: `the helper exited with ${r.exitCode}` }
  const line = String(r.output).split('\n').map((l) => l.trim()).filter((l) => l.startsWith('{"body":')).pop()
  if (!line) return { problem: 'its output holds no answer of the helper' }
  let out
  try { out = JSON.parse(line) } catch { return { problem: 'its output is not JSON' } }
  if (!out || typeof out !== 'object' || !out.body || typeof out.body !== 'object' || typeof out.hash !== 'string') return { problem: 'its output is not the helper\'s answer' }
  if (out.hash !== fnv1a(JSON.stringify(out.body))) return { problem: 'its output does not match its hash: the courier changed it' }
  if (out.body.received !== expected) return { problem: `the helper received another payload than the script sent${out.body.error ? `: ${out.body.error}` : ''}` }
  return { body: out.body }
}

let runId = null
let root = null
let branch = null
let laneDir = null
let current = 'Start'
const enter = (title) => {
  current = title
  phase(title)
}

// Op ids count per label and story, so that a story's steps get the same ids
// however the stories in flight interleave: a resumed run then gives a step
// the id its first run gave it, and the helper answers it from its record
// instead of writing it twice.
const opCounts = new Map()
const opIdOf = (base) => {
  const clean = base.replace(/[^A-Za-z0-9._-]+/g, '-')
  const n = (opCounts.get(clean) || 0) + 1
  opCounts.set(clean, n)
  return `${clean}-${n}`
}

// A courier that failed, and a step the helper says may work again (retry
// true: a payload that came wrong, a command that failed), are tried again
// with the same opId: the helper answers a write it made from its record,
// and carries on a write it left half done. A start that failed has given
// the lock up, so its repeat starts again. A step the helper refused for
// good (retry false: the board refused the write, another run holds the
// lock, a story worktree with no story/<key> to go back on) is not tried
// again.
const carry = async (name, prompt, expected, label, past = false) => {
  let problem = ''
  for (let i = 0; i <= COURIER_RETRIES; i++) {
    if (i) log(`${label}: ${problem}; trying again (${i} of ${COURIER_RETRIES})`)
    const { body, problem: wrong } = unpack(await ask('courier', prompt, { label, phase: current, schema: COURIER_SCHEMA, effort: 'low' }, MODELS.courier, past), expected)
    if (body && body.ok === true) return body
    if (body && body.retry !== true) throw new Error(`${label}: ${name} failed: ${body.error}`)
    problem = wrong || `${name} failed: ${body.error}`
  }
  throw new Error(`${label}: ${problem}, after ${COURIER_RETRIES} retries`)
}

// One host step. Its payload, with its opId, goes in a file under the run's
// folder, and the command carries the payload's hash.
const step = (name, fields = {}, { label = name, key = fields.key, past = false } = {}) => {
  const opId = opIdOf(key ? `${label}-${key}` : label)
  const payload = JSON.stringify({ opId, ...fields })
  const file = `${root}/.omni/runs/${runId}/in/${opId}.json`
  const command = `node ${shellPath(HELPER)} ${name} --run ${runId} --payload ${shellPath(file)} --hash ${fnv1a(payload)}`
  return carry(name, courierPrompt(command, file, payload), fnv1a(payload), key ? `courier:${name}:${key}` : `courier:${name}`, past)
}

// ---- prompts ---------------------------------------------------------------
// An omni command as an agent runs it, as this run and in the feature. The
// project needs no naming: omni finds the link of the main worktree from any
// git worktree of it.
const omniAs = (role) => `OMNI_RUN=${runId} OMNI_ROLE=${role} OMNI_FEATURE=${quote(feature)} omni`
const READ_ONLY = 'You are read-only: you change no file, make no commit, and write nothing to the board.'
const BOARD = `## The board
The workflow makes every write to the board itself, through its helper, from what you return. Never run an omni command that writes (a story claim, move, finding, decision, edit or block, a post, a thread create); reading with omni is fine.`

const decisionLines = (ctx) => ctx.decisions.map((d) => `- ${d.question} -> ${d.choice} (${d.by})`).join('\n') || '(none)'
const findingLine = (f) => `- [${f.stage}] ${f.text}${f.files.length ? ` (${f.files.join(', ')})` : ''}`
const byAttempt = (groups) => groups.map((g) => `Attempt ${g.attempt}:\n${g.findings.map(findingLine).join('\n')}`).join('\n')
const criteriaList = (ctx) => ctx.story.criteria.map((c) => `- ${c.text}`).join('\n') || '(none)'
const markedCriteria = (ctx) => ctx.story.criteria.map((c) => `- ${c.text}${c.human_check ? ' [human check]' : ''}`).join('\n') || '(none)'
// A criterion that names a test is a test obligation, which the lane's test
// author owns. Clarity records it as this decision, and a story clarity has
// checked has its obligations from those decisions alone: "The Test
// connection button shows Connected" is behaviour. A story implemented
// before clarity ran has none, so for it the script looks in the criterion
// itself for a test as the thing delivered: a kind of test, a test for or
// that, tests to add or write, or tests that pass or cover.
const testQuestion = (criterion) => `Is "${criterion}" behaviour or a test?`
const TEST_OBLIGATION = 'treated as a test obligation: the lane\'s test author owns it, and the verifier skips it'
const NAMES_TEST = /^(an?|the)\s+(unit|integration|e2e|regression)\s+tests?\b|^(an?|the)\s+tests?\s+(for|that|of)\b|\b(add|write)s?\b[^.]{0,30}\btests?\b|\btests?\s+(pass|cover)/i
const obligationsOf = (ctx) => new Set(ctx.story.criteria.flatMap((c, i) => (ctx.decisions.some((d) => d.question === testQuestion(c.text) && d.choice === TEST_OBLIGATION) || (!ctx.clarified && NAMES_TEST.test(c.text)) ? [i] : [])))
const numberedCriteria = (ctx) => {
  const owed = obligationsOf(ctx)
  return ctx.story.criteria.map((c, i) => `${i}: ${c.text}${c.human_check ? ' [human check]' : ''}${owed.has(i) ? ' [test obligation]' : ''}`).join('\n') || '(none)'
}
const storyText = (ctx) => `${ctx.story.key}: ${ctx.story.title}\n${ctx.story.text}`
const linkedNodes = (ctx) => ctx.docs.map((d) => `- doc ${d.doc}: ${d.nodes.map((n) => `node ${n.node}${n.label ? ` (${n.label})` : ''}`).join(', ')}`).join('\n')
// Doc text never goes through a courier: each agent reads each linked doc
// once itself, and is told which nodes the story links.
const docsSection = (ctx, role) => (ctx.docs.length
  ? `## The linked docs
Read each doc below once, with \`${omniAs(role)} doc read <doc>\`, and find in it the nodes the story links: each is the element with that data-omni-id.
${linkedNodes(ctx)}`
  : '## The linked docs\n(none)')

const TWO_IMPLEMENTATIONS = `
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
`
const BLOCKING_OR_MINOR = `
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
  option; it will be built and verified as if the spec said so, and recorded as an
  agent decision for the human to see.
`
const TESTS_AND_IMPACT = `
## A criterion that names a test
A criterion that names a test (a test, test file, suite or case to write or to pass)
does not describe outside behaviour. List it in testObligations, verbatim, and raise
no gap for it: the workflow records it as a test obligation, which the lane's test
author owns and the verifier skips.

## Impact (decide for every gap)
Say what the answer decides: scope (what the story does at all), data_security (what
is stored, sent or deleted; permissions, security or money), ux (what the user sees
or can do), or technical (an API contract or other technical behaviour). The
workflow asks the human at most ${CLARITY_QUESTIONS} blocking questions per story, the highest impact
first, and takes your recommendation for the rest.
`

const CLARITY_SCHEMA = {
  type: 'object',
  required: ['testObligations', 'gaps'],
  properties: {
    testObligations: { type: 'array', items: { type: 'string' }, description: 'The criteria that name a test instead of outside behaviour, verbatim' },
    gaps: {
      type: 'array',
      description: 'Criteria in this story that fail the two-implementation test. Empty when every criterion decides the behaviour',
      items: {
        type: 'object',
        required: ['criterion', 'question', 'options', 'recommendation', 'reason', 'blocking', 'impact'],
        properties: {
          blocking: { type: 'boolean', description: 'true only for a blocking gap per the rule; a minor gap is decided by your recommendation' },
          impact: { type: 'string', enum: ['scope', 'data_security', 'ux', 'technical'], description: 'What the answer decides, per the rule' },
          criterion: { type: 'string', description: 'The criterion text, verbatim' },
          question: { type: 'string', description: 'The decision the spec leaves open, as one plain question' },
          options: { type: 'array', items: { type: 'string' }, minItems: 2, description: 'Concrete behaviours, each one a reasonable implementation would show' },
          recommendation: { type: 'string', description: 'The option you suggest' },
          reason: { type: 'string', description: 'The spec text behind the recommendation, quoted' },
        },
      },
    },
  },
}
const clarityPrompt = (ctx) => `You check ONE story of the feature "${feature}" for gaps before any code is written. ${READ_ONLY}

## The story
${storyText(ctx)}

### Files planned
${ctx.story.files_planned.join('\n') || '(none named)'}

### Blocked by
${ctx.story.blocked_by.join(', ') || '(none)'}

### Recorded decisions
They are settled: a gap one of them already settles is NOT raised again.
${decisionLines(ctx)}

## The shared parts
The rule below refers to the feature's Spec, Architecture, DB and UI docs. List them with \`${omniAs('clarity')} doc list\` and read each once with \`${omniAs('clarity')} doc read <doc>\`. Treat them as the shared parts of every story: a criterion they already settle is not a gap.${ctx.docs.length ? ` The story links these nodes of them (each is the element with that data-omni-id):\n${linkedNodes(ctx)}` : ''}

${BOARD}
${TWO_IMPLEMENTATIONS}${BLOCKING_OR_MINOR}${TESTS_AND_IMPACT}
## Acceptance criteria
${markedCriteria(ctx)}

## Your task now
Apply the rules above to every criterion, and return the test obligations and the gaps.
Return raw structured data only.`

const IMPL_SCHEMA = {
  type: 'object',
  required: ['status', 'summary'],
  properties: {
    status: { type: 'string', enum: ['ready', 'blocked', 'needs_decision'] },
    summary: { type: 'string', description: '3-6 sentences: what was built and how. For blocked: what blocks you' },
    decisions: {
      type: 'array',
      description: 'Non-obvious technical decisions you took',
      items: { type: 'object', required: ['question', 'choice'], properties: { question: { type: 'string' }, choice: { type: 'string' } } },
    },
    commits: { type: 'array', items: { type: 'string' }, description: 'Commit hashes you made, oldest first' },
    question: { type: 'string', description: 'Only with needs_decision: the product decision the story cannot avoid, as one plain question' },
    options: { type: 'array', items: { type: 'string' }, description: 'Only with needs_decision: the concrete options' },
    recommendation: { type: 'string', description: 'Only with needs_decision: the option you suggest' },
  },
}
// The last line says where to commit: the implementer on its story branch,
// the test author on the lane's detached HEAD.
const conventions = (where) => `
## Repo conventions (MUST follow)
- Commit messages: "type: Capitalized subject" (e.g. "feat: Add booking list").
  NO parenthetical scope. End the message body with the Co-Authored-By line
  from your own commit-attribution reminder, copied exactly (it names the model
  you are running on, which differs between roles).
- Stage files EXPLICITLY with git add <paths>. NEVER git add -A / git add .
- NEVER stage or commit: anything under tasks/ (PRDs are local planning artifacts),
  anything under .claude/, scratch or debug files, service-level CLAUDE.md files.
- Do not add extra whitespace for column alignment in code.
- Match the surrounding code's style, comment density, and idiom.
- Do NOT create notes, plan, progress, or memory files anywhere in the repo.
  Everything you learned or decided goes in your structured return value.
  Temp/debug files go in your scratchpad directory, never the repo.
- ${where}
`
const CONVENTIONS = conventions('Work on the CURRENT branch. Do not push, create branches or amend.')
// The earlier findings of the story, by attempt: those since the human's
// latest answer to fix, those that used no attempt, and those before the
// answer for what failed before it.
const findingsSection = (ctx, fixing) => [
  ctx.findings.length ? `\n\n### Findings of the earlier attempts (${fixing})\n${byAttempt(ctx.findings)}` : '',
  ctx.uncounted.length ? `\n\n### Other open findings (${fixing}; they used no attempt)\n${ctx.uncounted.map(findingLine).join('\n')}` : '',
  ctx.earlier.length ? `\n\n### Findings before the human's latest answer (what failed before it)\n${byAttempt(ctx.earlier)}` : '',
].join('')
const FRESH = 'Implement the story: build what the criteria above require, commit it, and return the result.'
const CONTINUE = 'A worktree of an earlier attempt is in place, with its commits and what it left uncommitted: continue from it, do not start over, and fix every finding above.'
const RESUME = 'An earlier session of this attempt stopped to ask a question, which the decisions above now settle. What it committed or left uncommitted is in the worktree: continue from it, do not start over.'
// found: what the worktree step found when the worktree was not on
// story/<key>, and what to do about it.
const implPrompt = (ctx, wt, attempt, task, found) => `You implement ONE story of the feature "${feature}" in its own git worktree.

## Your worktree
${wt} (branch story/${ctx.story.key}, cut from ${branch}). Work only there: cd into it first, and run every command there.${found ? `\n${found}` : ''}

## The story
${storyText(ctx)}

### Files planned
${ctx.story.files_planned.join('\n') || '(none named)'}

### Decisions already taken (settled: build against them as if the spec said so)
${decisionLines(ctx)}${findingsSection(ctx, 'fix every one of them')}

${docsSection(ctx, 'impl')}

${BOARD}
${CONVENTIONS}
## Rules
- Implement only this story. No unrelated cleanup, no groundwork for later stories.
- Commit a feat: commit, then fix: commits for later corrections. Commit the story's production files.
- Write NO tests: no new test, test file, test helper, fixture or snapshot. The lane's test author writes the story's tests from the story text, by the writing-tests skill, after a verifier has confirmed the story works; a criterion that a decision treats as a test obligation is theirs. The one exception: an existing test that asserts the behaviour this story changes may be updated to the story's decision; name it in decisions.
- Before you finish, run the narrowest existing check that proves your change: one crate's existing tests, one package's typecheck. Do not run the full check command: the lane runs it, in a worktree of its own, after the review.
- Never start the stack or run the stack tests${checks.stack ? ` (\`${checks.stack}\`)` : ''}, and never run \`docker compose up\` or \`docker compose run\`: the stack is shared, and the lane runs it on the story after the review. When your change needs the stack to be checked, say so in decisions.
- If the story needs a product decision the spec does not make and the story cannot avoid, STOP: status needs_decision with question, options and recommendation. Never guess a product rule. Ordinary technical choices are yours.
- If you genuinely cannot continue, status blocked with the reason in summary.
- Return decisions for every non-obvious technical choice.

## Acceptance criteria
${criteriaList(ctx)}

## Your task now
This is attempt ${attempt} of ${MAX_ATTEMPTS} since the human's latest answer. ${task}
Return raw structured data only.`

// Every criterion gets an entry, numbered from 0 as the prompt numbers them.
const reviewSchema = (n) => ({
  type: 'object',
  required: ['verdict', 'criteria', 'defects'],
  properties: {
    verdict: { type: 'string', enum: ['pass', 'fail', 'needs_decision'] },
    criteria: {
      type: 'array',
      minItems: n,
      maxItems: n,
      description: 'One entry for every acceptance criterion of the story, by its number counting from 0',
      items: {
        type: 'object',
        required: ['index', 'criterion', 'obligation', 'met', 'evidence'],
        properties: {
          index: { type: 'integer', minimum: 0, maximum: Math.max(n - 1, 0), description: 'The number of the criterion in the list, counting from 0' },
          criterion: { type: 'string', description: 'The criterion text, verbatim' },
          obligation: { type: 'boolean', description: 'true for a criterion marked [test obligation], which you do not judge; false for every other' },
          met: { type: 'boolean', description: 'Whether the code meets the criterion; not read for a test obligation' },
          evidence: { type: 'string', description: 'What the code does about it, with path:line; for a criterion not met, what is wrong and where' },
        },
      },
    },
    defects: {
      type: 'array',
      description: 'Every defect you found, one per problem, not only the first. Empty on pass',
      items: {
        type: 'object',
        required: ['text', 'files', 'criterion'],
        properties: {
          text: { type: 'string', description: 'What is wrong, what the story requires, and where' },
          files: { type: 'array', items: { type: 'string' } },
          criterion: { type: 'string', description: 'The acceptance criterion it breaks, verbatim; empty when it breaks an interface or doc node' },
        },
      },
    },
    question: { type: 'string', description: 'Only with needs_decision: the product decision the story cannot avoid, as one plain question' },
    options: { type: 'array', items: { type: 'string' }, description: 'Only with needs_decision: the concrete options' },
    recommendation: { type: 'string', description: 'Only with needs_decision: the option you suggest' },
  },
})
const reviewPrompt = (ctx, wt) => `You review ONE story of the feature "${feature}" before it goes to the test lane. ${READ_ONLY} You run no test, build or app either.

## The change
Worktree ${wt}, branch story/${ctx.story.key}, cut from ${branch}. See the diff with \`git -C ${wt} diff refs/heads/${branch}...HEAD\` and read the changed files there.

## The story
${storyText(ctx)}

### Files planned
${ctx.story.files_planned.join('\n') || '(none named)'}

### Decisions already taken (settled: judge against them as if the spec said so)
${decisionLines(ctx)}${findingsSection(ctx, 'check that each is fixed')}

${docsSection(ctx, 'review')}

${BOARD}

## Rules
- Read the diff and the files against every criterion, the interfaces and the linked doc nodes.
- A new test, test file, test helper, fixture or snapshot in the diff is a defect: the implementer writes no tests, the lane's test author does. An existing test updated to a decision the story names is fine. A missing test is never a defect.
- A criterion marked [test obligation] is a test obligation: the lane's test author owns it, and the review never judges it. Give it obligation true, and every other criterion obligation false: every other criterion is judged.
- A [human check] criterion: met unless the code contradicts it; a human confirms it later.
- Give every criterion below an entry, by its number counting from 0: met or not, with the evidence, or obligation true for a test obligation.
- List every defect you find, not only the first: one per problem, with the files it is in and the criterion it breaks.
- verdict pass: every criterion that is not a test obligation is met, and you found no defect.
- verdict fail: a criterion is not met, or you found a defect.
- verdict needs_decision: only when the spec does not decide the behaviour and the story cannot avoid it; give question, options and recommendation.

## Acceptance criteria (number: text)
${numberedCriteria(ctx)}

## Your task now
Review the change against every criterion above.
Return raw structured data only.`

// ---- the lane's agents ---------------------------------------------------------
// They work in the lane worktree, on the detached HEAD lane-prepare checked
// out: a branch checked out there would move with each commit, past the
// lane's compare-and-swap.
const LANE_HEAD = 'Stay on the detached HEAD: never check out or create a branch.'
const VERIFY_SCHEMA = {
  type: 'object',
  required: ['criteria'],
  properties: {
    criteria: {
      type: 'array',
      description: 'One entry per acceptance criterion in the story, by its number counting from 0',
      items: {
        type: 'object',
        required: ['index', 'criterion', 'ok', 'evidence'],
        properties: {
          index: { type: 'integer', description: 'The number of the criterion in the list, counting from 0' },
          criterion: { type: 'string', description: 'The criterion text, verbatim' },
          ok: { type: 'boolean' },
          evidence: { type: 'string', description: 'The command or steps, the input, and what you actually observed. For a failure: what is wrong and where' },
        },
      },
    },
  },
}
// The project's commands a criterion about one is checked by; those unset
// are left out.
const commandsOf = () => ['fast', 'stack', 'lint'].filter((name) => checks[name]).map((name) => `${name} is \`${checks[name]}\``)
const verifyPrompt = (ctx) => `You check ONE story of the feature "${feature}" by running it. ${READ_ONLY}

## Where
The lane worktree ${laneDir} holds story/${ctx.story.key} rebased onto ${branch}, on a detached HEAD, and the story's checks passed there${checks.stack_up ? `; the stack is up, as \`${checks.stack_up}\` started it from there. Exercise the story against that stack, not another copy on the machine` : ''}. Work only there: cd into it first, and run every command there. ${LANE_HEAD}

## The story
${storyText(ctx)}

### Decisions already taken (settled: judge against them as if the spec said so)
${decisionLines(ctx)}

${docsSection(ctx, 'verify')}

${BOARD}

## Rules
- Exercise EVERY criterion by running the behaviour in ${laneDir}: call the endpoint, run the command, open the page with the dev-browser skill for browser criteria. Reading code is not a check.
- A criterion marked [test obligation] is the lane's test author's: skip it, with ok true and the evidence "a test obligation".
- The stack has no users until the seed runs${checks.seed ? `. For a credential, run the seed command as \`OMNI_STORY=${ctx.story.key} OMNI_RUN=${runId} sh -c ${quote(checks.seed)}\` in ${laneDir}: it prints what to sign in with` : ', and this run has no seed command'}. Never write into the stack's database yourself.
${commandsOf().length ? `- A criterion about a command is checked by running that command: ${commandsOf().join(', ')}.\n` : ''}- A [human check] criterion: exercise what you can; ok is true unless what you ran contradicts it. A human confirms it later.
- ok false only for a criterion the behaviour does not meet. Say what is wrong and where.

## Acceptance criteria (number: text)
${numberedCriteria(ctx)}

## Your task now
Exercise every criterion above, and return one entry for each, by its number.
Return raw structured data only.`

const TEST_SKILL = '~/.claude/skills/writing-tests/SKILL.md'
const TEST_ASSETS = `test and spec files, test helpers, fixtures, snapshots, test-runner,
coverage and lint configuration, and skip/only/todo markers`
const TEST_POLICY = `
## Regression-test policy (MUST follow)
- FIRST read ${TEST_SKILL} and the reference files it points to as you reach them.
  Follow it exactly; the story below and its linked doc nodes are its spec, and the
  verifier's list of confirmed criteria tells you which behaviours were confirmed.
- The story already WORKS: a verifier exercised it. You are not proving it works.
  You are choosing the few tests that would catch it breaking later. Criteria are not
  test cases: one test per criterion is the pattern this replaces, and a criterion
  that states a detail with no rule behind it gets no test (skill step 1).
- NO TARGET COUNT: rank the oracle rows (skill step 1) and admit tests in rank order
  through the gate (skill step 8). Zero is a correct answer when existing tests
  already catch everything. A row you rank but do not write, because a test already
  catches it, goes in notAdmitted with the test that catches it. List added tests in
  rank order, highest first.
- The implementer wrote no tests for this story: every test of its behaviour is yours
  to rank and admit, and the inventory tells you what earlier tests already catch.
- Tests added by earlier stories in this run are part of the existing suite.
  Using earlier stories' features as setup is fine; do not re-test their rules.
`
const TESTS_SCHEMA = {
  type: 'object',
  required: ['status', 'commit', 'check', 'oracle', 'existingCoverage', 'added', 'reused', 'changedOrRemoved', 'notAdmitted'],
  properties: {
    status: { type: 'string', enum: ['done', 'found_defect', 'needs_decision', 'blocked'] },
    question: { type: 'string', description: 'Only with needs_decision: what the story does not settle, as one plain question' },
    options: { type: 'array', items: { type: 'string' }, description: 'Only with needs_decision: the concrete options' },
    recommendation: { type: 'string', description: 'Only with needs_decision: the option you suggest' },
    commit: { type: 'string', description: 'The test: commit hash, or empty string when no test change was needed' },
    check: { type: 'string', description: 'The one command, run from the lane worktree, that runs exactly the tests you added (e.g. cargo test -p omni-core --test design_tokens, or pnpm --filter web exec vitest run src/x.test.tsx); empty when you added nothing' },
    oracle: {
      type: 'array',
      description: 'The oracle table (skill step 1), written from the story before reading the implementation: one row per key example',
      items: {
        type: 'object',
        required: ['input', 'expected', 'clause'],
        properties: {
          input: { type: 'string' },
          expected: { type: 'string' },
          clause: { type: 'string', description: 'The story text the expected value comes from' },
        },
      },
    },
    existingCoverage: {
      type: 'array',
      description: 'Inventory made BEFORE writing anything: every existing test that exercises this story\'s behaviour, and what it already pins',
      items: { type: 'object', required: ['test', 'file', 'covers'], properties: { test: { type: 'string' }, file: { type: 'string' }, covers: { type: 'string' } } },
    },
    added: {
      type: 'array',
      items: {
        type: 'object',
        required: ['test', 'file', 'risk', 'overlapCheck', 'sensitivity'],
        properties: {
          test: { type: 'string' },
          file: { type: 'string' },
          risk: { type: 'string', description: 'What breakage this catches, and the story line its expected values come from' },
          overlapCheck: { type: 'string', description: 'The closest existing tests you compared it with, and why none of them already catches this breakage' },
          sensitivity: { type: 'string', description: 'Each mutant applied (file, line, change), that this test went red on it, and the existing tests run against it that stayed green' },
        },
      },
    },
    reused: { type: 'array', items: { type: 'object', required: ['test', 'risk'], properties: { test: { type: 'string' }, risk: { type: 'string' } } }, description: 'Existing tests that already protect a risk, so none was added' },
    changedOrRemoved: { type: 'array', items: { type: 'object', required: ['test', 'action', 'reason'], properties: { test: { type: 'string' }, action: { type: 'string', enum: ['changed', 'removed'] }, reason: { type: 'string' } } } },
    notAdmitted: {
      type: 'array',
      description: 'Ranked oracle rows you did not write a test for, because another test already catches them',
      items: { type: 'object', required: ['row', 'caughtBy'], properties: { row: { type: 'string' }, caughtBy: { type: 'string', description: 'The existing or newly added test that already catches it' } } },
    },
    defect: {
      type: 'object',
      properties: { test: { type: 'string' }, observed: { type: 'string' }, expected: { type: 'string' } },
      description: 'With status found_defect: the behaviour your test exposed as wrong against the story',
    },
    blockers: { type: 'string' },
    learnings: { type: 'array', items: { type: 'string' } },
  },
}
// The tests this run's test authors added, for the authors after them.
const testsAdded = []
const WRITE_TESTS = 'Write the story\'s regression tests by the policy above, commit them, and return the result.'
const RESUME_TESTS = 'An earlier session stopped to ask a question, which the decisions above now settle. What it committed is in the lane worktree: continue from it, do not start over.'
// The author asked once more, after its check came out red.
const fixTests = (check, r) => `An earlier session committed tests in the lane worktree, and their check \`${check}\` ${ended(r)}
The story itself passed its checks and its verifier. Fix the tests, never the production code, so that the check passes, commit the fix, and return the result with the check.`
const authorPrompt = (ctx, base, confirmed, task) => `You write the lasting regression tests for one story of the feature "${feature}", with a fresh context.

## Your worktree
The lane worktree ${laneDir} holds story/${ctx.story.key} rebased onto ${branch}, on a detached HEAD, and a verifier has confirmed there that it matches its criteria. Work only there: cd into it first, and run every command there. ${LANE_HEAD} Commit on that HEAD: the workflow lands your commits. You own tests; you never change production code.

## The story
${storyText(ctx)}

### Decisions already taken (settled: test against them as if the spec said so)
${decisionLines(ctx)}

${docsSection(ctx, 'tests')}

## Criteria the verifier confirmed (observed values are withheld so they cannot shape your oracle)
${confirmed.map((c) => `- ${c}`).join('\n') || '(none)'}
${testsAdded.length ? `\n## Tests added earlier in this run (part of the existing suite; do not re-test what they pin)\n${testsAdded.map((x) => `- ${x}`).join('\n')}\n` : ''}
${BOARD}
${conventions('Commit on the detached HEAD of the lane worktree. Do not push, check out or create a branch, or amend.')}${TEST_POLICY}
## Rules
- Record \`git rev-parse HEAD\` first. You may change only ${TEST_ASSETS}.
- A criterion marked [test obligation] names a test the story owes: the verifier skipped it, and it is yours. Meet it with a test the gate admits, or name in reused the existing test that already does.
- ORACLE FIRST (skill step 1): write the oracle table from the story text above BEFORE you open any production file or run the code. Return it in oracle. Every expected value in your tests comes from a row. If the code disagrees with a row and the story clearly supports the row, that is a defect (status found_defect); if the story does not settle it, return status needs_decision with question, options and recommendation. In both cases leave no uncommitted test changes.
- INVENTORY (skill step 2), before writing any test. Besides the skill's search, search by the names in the story's files and by what \`git diff ${base}..HEAD\` touches. Record each existing test found in existingCoverage, and for each new test the closest existing ones in overlapCheck.
- PROVE IT GOES RED (skill step 6) for every new test: record in sensitivity each mutant you applied, that the new test went red, and which existing tests you ran against it that stayed green. Production files must be unchanged when you finish (git status shows no production changes).
- Before you commit, run the test files you touched and the typecheck of their crate or package, not the full suite: the lane ran its checks on this commit before you started, and runs your new tests once more after you with the command you return in check. Leave the working tree clean: no mutant left in a production file, no uncommitted test file. Commit only test files: "test: <summary>". If you add nothing, commit nothing and return an empty commit and an empty check.
- If you genuinely cannot continue, status blocked with the reason in blockers.

## Acceptance criteria (number: text)
${numberedCriteria(ctx)}

## Your task now
${task}
Return raw structured data only.`

const FINAL_SCHEMA = {
  type: 'object',
  required: ['healthy', 'journeys', 'report'],
  properties: {
    healthy: { type: 'boolean', description: 'The fast command passed and every cross-story journey worked' },
    journeys: { type: 'array', items: { type: 'string' }, description: 'Each cross-story journey you exercised, with the outcome' },
    report: { type: 'string' },
  },
}
const finalPrompt = (landed) => `You run the final check after a delivery run on the feature "${feature}". ${READ_ONLY}

## Where
The lane worktree ${laneDir}, on a detached HEAD at the tip of ${branch}, where this run landed its stories. Work only there: cd into it first, and run every command there. ${LANE_HEAD}

## The stories this run landed
${landed.map((key) => `- ${key}`).join('\n')}

${BOARD}

## Your task now
1. ${checks.fast ? `Run the fast command \`${checks.fast}\` in ${laneDir}.` : 'The project has no fast command: go on to 2.'}
2. Each story was checked on its own. Find the user journeys in the feature's docs (\`${omniAs('final')} doc list\`, then \`${omniAs('final')} doc read <doc>\`) and in these stories (\`${omniAs('final')} story read <key>\`) that span several of them (one story's output is another's input, a flow through several screens or endpoints), and exercise up to 3 of them for real, with the real components${checks.stack ? `, using the stack command \`${checks.stack}\`` : ''}. Report each with its outcome.
3. Stop every process and remove every container you started for the journeys, and confirm none is left.
Do NOT change any code. healthy is false if the fast command fails or a journey fails; a failure is reported, not repaired.
Return raw structured data only.`

// ---- the council -----------------------------------------------------------
// Three members, each reading one thing, answer a question with options on
// their own, in one round: agents that talk it over give up right answers to
// agree.
const READINGS = [
  { id: 'spec', name: 'The spec docs and decisions' },
  { id: 'code', name: 'The code' },
  { id: 'user', name: 'What the user would expect' },
]
const VOTE_SCHEMA = {
  type: 'object',
  required: ['pick', 'quote', 'source', 'minor'],
  properties: {
    pick: { type: 'integer', minimum: 0, description: 'The number of the option your reading supports, or 0 for none' },
    quote: { type: 'string', description: 'The words that support your pick, quoted exactly; empty when you have no evidence' },
    source: { type: 'string', description: 'Where the quote is: doc <id> node <id>, the decision "<question>", or path:line; empty when you have no evidence' },
    minor: { type: 'boolean', description: 'true when the gap is minor under the rule' },
    why: { type: 'string', description: 'One sentence: why your reading supports that option, or none' },
  },
}
const readingText = (reading, ctx, code) => {
  if (reading.id === 'spec') {
    return `You read the spec, and nothing else: the story's linked docs and the feature's decisions.${ctx.docs.length ? ` Read each doc below once, with \`${omniAs('council')} doc read <doc>\`, and find in it the nodes the story links (each is the element with that data-omni-id):\n${linkedNodes(ctx)}\n` : ' The story links no doc node. '}The feature's decisions are those on its stories: \`${omniAs('council')} story list --json\` gives every story with its decisions. Pick what they say; when they say nothing about the question, pick none.`
  }
  if (reading.id === 'code') {
    return `You read the code, and nothing else: ${code ? `the story's worktree ${code}` : `the tip of ${branch}, with \`git -C ${root} grep -n <pattern> refs/heads/${branch}\` and \`git -C ${root} show refs/heads/${branch}:<path>\``}. Pick what the code already does or plainly expects: an existing behaviour, a convention, an interface. Quote it, with path:line as its source.`
  }
  return 'You judge as the feature\'s user, and nothing else: which option would surprise them least? The story says who they are and what they are doing. Quote a doc node or a decision only when one says so; otherwise give no evidence.'
}
const councilPrompt = (ctx, q, reading, code) => `You are one member of a council of three that settles a question a story raised, so that the human is asked only what the spec, the code and the user's expectations leave open. ${READ_ONLY} You answer on your own: the other members never see your answer, and there is one round.

## The story
${storyText(ctx)}

### Acceptance criteria
${criteriaList(ctx)}

### Decisions already taken
${decisionLines(ctx)}

## Your reading
${readingText(reading, ctx, code)}

${BOARD}
${BLOCKING_OR_MINOR}
## The question
${q.question}
${q.criterion ? `Criterion: ${q.criterion}\n` : ''}Asked by ${q.asker}, who recommends: ${q.recommendation || '(nothing)'}${q.reason ? `. Why: ${q.reason}` : ''}

Options:
${(q.options || []).map((o, i) => `${i + 1}. ${o}`).join('\n') || '(none given)'}

## Your task now
Pick the option your reading supports, or 0 when it supports none. Give your evidence: the words that support the pick, quoted exactly, with their source (doc <id> node <id>, the decision "<question>", or path:line); leave quote and source empty when you have none. Say whether the gap is minor under the rule above.
Return raw structured data only.`

const councilSlots = semaphore(COUNCILS)
// For the run's report: questions the council settled, and those sent to the human.
const tally = { settled: 0, asked: 0 }
// A question for the human: a thread named after the story, with the
// options and a recommendation, linked from the story, which is blocked
// until it concludes.
const askHuman = async (ctx, q, label, recommendation, council) => {
  const key = ctx.story.key
  const body = [
    q.question,
    ...(q.criterion ? [`Criterion: ${q.criterion}`] : []),
    `Asked by ${q.asker}.`,
    `Options:\n${(q.options || []).map((o, i) => `${i + 1}. ${o}`).join('\n') || '(none given)'}`,
    `The council: ${council}`,
    `Recommendation: ${recommendation}`,
    ...(q.reason ? [`Why: ${q.reason}`] : []),
  ].join('\n\n')
  const { thread } = await step('thread-open', { key, title: `${key}: ${q.question}`, body: `${body}\n` }, { label: `${label}-thread` })
  tally.asked++
  log(`${key}: asked the human "${q.question}" on thread ${thread}`)
  return { settled: false, thread }
}
// The members' votes, in one round; a member that died votes nothing.
const convene = (ctx, q, code) => councilSlots.run(() => parallel(READINGS.map((reading) => () =>
  ask('council', councilPrompt(ctx, q, reading, code), { label: `council:${ctx.story.key}:${reading.id}`, phase: current, schema: VOTE_SCHEMA }))))
// The council settles a question when all three pick the same option and a
// quote supports it, or all three call the gap minor: it becomes an agent
// decision and no thread opens. Otherwise the thread opens with each
// member's pick and evidence, and the majority's pick, else the asker's, as
// its recommendation.
const conclude = async (ctx, q, votes, label) => {
  const key = ctx.story.key
  const options = q.options || []
  const picks = votes.map((v) => (v && Number.isInteger(v.pick) && v.pick >= 1 && v.pick <= options.length ? v.pick : 0))
  const evidenced = votes.filter((v) => v && String(v.quote || '').trim() && String(v.source || '').trim())
  if (picks[0] && picks.every((p) => p === picks[0]) && (evidenced.length || votes.every((v) => v && v.minor === true))) {
    const evidence = evidenced.length ? evidenced.map((v) => `"${v.quote.trim()}" (${v.source.trim()})`).join('; ') : 'all three members call the gap minor'
    await step('decision', { key, question: q.question, choice: `${options[picks[0] - 1]} (council: ${evidence})`, by: 'agent' }, { label: `${label}-decision` })
    tally.settled++
    log(`${key}: the council settled "${q.question}"`)
    return { settled: true }
  }
  const majority = picks.find((p) => p && picks.filter((x) => x === p).length >= 2)
  const members = READINGS.map((reading, i) => {
    const v = votes[i]
    if (!v) return `- ${reading.name}: returned nothing`
    const evidence = String(v.quote || '').trim() && String(v.source || '').trim() ? `"${v.quote.trim()}" (${v.source.trim()})` : 'no evidence'
    return `- ${reading.name}: ${picks[i] ? `option ${picks[i]}, ${options[picks[i] - 1]}` : 'no option'}; evidence: ${evidence}; minor: ${v.minor === true ? 'yes' : 'no'}${v.why ? `. ${v.why}` : ''}`
  })
  const recommendation = majority ? `${options[majority - 1]} (the council's majority)` : `${q.recommendation || '(none)'} (the recommendation of ${q.asker})`
  return askHuman(ctx, q, label, recommendation, `it did not settle this, in one round with each member on its own:\n${members.join('\n')}`)
}
const settle = async (ctx, q, { code, label }) => conclude(ctx, q, await convene(ctx, q, code), label)
// A question of an implementer or reviewer: to the council, or past the
// questions it settles in one attempt, to the human.
const question = (ctx, r, asker, code, label, asked) => {
  const q = { question: r.question || '(no question given)', options: r.options || [], recommendation: r.recommendation || '', asker }
  if (asked < QUESTIONS_PER_ATTEMPT) return settle(ctx, q, { code, label })
  return askHuman(ctx, q, label, `${q.recommendation || '(none)'} (the recommendation of ${asker})`, `not asked: it settled ${QUESTIONS_PER_ATTEMPT} questions of ${asker} in this attempt already`)
}

// ---- 1. start ----------------------------------------------------------------
// start has no run or root yet, so its small payload goes as flags.
enter('Start')
const force = A.force === true
const startOp = opIdOf('start')
const startHash = fnv1a(JSON.stringify({ opId: startOp, feature, force }))
const started = await carry('start', courierPrompt(`node ${shellPath(HELPER)} start --op ${startOp} --feature ${quote(feature)}${force ? ' --force' : ''} --hash ${startHash}`), startHash, 'courier:start')
runId = started.run
root = started.root
branch = started.branch
laneDir = started.lane
for (const line of started.log || []) log(line)
log(`Run ${runId} on ${feature}: stories land on ${branch}, now at ${String(started.tip).slice(0, 7)}`)
let checks = started.checks
// The tip of deliver/<slug>: where start left it, then where each story landed.
let tip = started.tip
// From here on every step runs in the try at the end, whose finally finishes
// the run: it posts the report and gives the lock up, after a throw too.

// ---- 2. unblock --------------------------------------------------------------
// A thread's conclusion is the human's answer, and restarts the story's attempts.
let unblocked = { unblocked: [] }
const unblock = async () => {
  unblocked = await step('unblock-pass', {})
  for (const line of unblocked.log || []) log(line)
  if (unblocked.unblocked.length) log(`Unblocked ${unblocked.unblocked.join(', ')}`)
}

// ---- 3. clarity --------------------------------------------------------------
const IMPACT = ['scope', 'data_security', 'ux', 'technical']
const clarity = []
// Stories whose clarity check returned nothing, or failed: not delivered in
// this run, and checked again at the next start.
const unclear = new Set()
// Clarity checks in flight, which the agent cap sets calls aside for.
let checking = 0
const clarify = async (key) => {
  if (!room(CHECK_CALLS, checking)) {
    unclear.add(key)
    capped = true
    log(`${key}: not checked: the run has made ${agentCalls} agent calls, with ${checking} clarity checks in flight, and keeps the rest for them; the story is checked at the next start`)
    return
  }
  checking++
  try {
    await check(key)
  } finally {
    checking--
  }
}
const check = async (key) => {
  const ctx = await step('context', { key })
  const r = await ask('clarity', clarityPrompt(ctx), { label: `clarity:${key}`, phase: 'Clarity', schema: CLARITY_SCHEMA })
  if (!r) {
    unclear.add(key)
    log(`${key}: the clarity check returned nothing; the story is not delivered in this run, and is checked again at the next start`)
    return
  }
  // A question the story has a decision on is settled.
  const known = new Set(ctx.decisions.map((d) => d.question))
  const decide = async (q, choice) => {
    if (known.has(q)) return
    known.add(q)
    await step('decision', { key, question: q, choice, by: 'agent' })
  }
  for (const criterion of r.testObligations || []) await decide(testQuestion(criterion), TEST_OBLIGATION)
  const gaps = (r.gaps || []).filter((g) => !known.has(g.question))
  for (const g of gaps.filter((x) => !x.blocking)) await decide(g.question, g.recommendation)
  const rank = (g) => (IMPACT.includes(g.impact) ? IMPACT.indexOf(g.impact) : IMPACT.length)
  const blocking = gaps.filter((x) => x.blocking).sort((a, b) => rank(a) - rank(b))
  for (const g of blocking.slice(CLARITY_QUESTIONS)) await decide(g.question, `${g.recommendation} (deferred by the cap: a story asks at most ${CLARITY_QUESTIONS} blocking questions)`)
  // The councils sit side by side, within the run's limit, and every one is
  // waited for. Their answers go to the board one at a time, in order: two
  // thread-opens of one story at once could each link the story without the
  // other's thread.
  const asked = blocking.slice(0, CLARITY_QUESTIONS).map((g) => ({ ...g, asker: 'the clarity check' }))
  const ballots = await Promise.all(asked.map((q) => convene(ctx, q, null).then((votes) => ({ votes }), (error) => ({ error }))))
  const answers = []
  const failures = []
  for (const [i, q] of asked.entries()) {
    try {
      if (ballots[i].error) throw ballots[i].error
      answers.push(await conclude(ctx, q, ballots[i].votes, `gap${i + 1}`))
    } catch (e) {
      if (e && e.atCap) throw e
      failures.push(`"${q.question}": ${(e && e.message) || e}`)
    }
  }
  // A question neither settled nor asked keeps the story unrecorded, so that
  // the next start checks it again.
  if (failures.length) throw new Error(`a question was neither settled nor asked: ${failures.join('; ')}`)
  // Recorded after the check's own writes, so that they do not make the
  // story pending again.
  await step('clarity-record', { key })
  const threads = answers.filter((a) => !a.settled).map((a) => a.thread)
  clarity.push({ key, obligations: (r.testObligations || []).length, minor: gaps.filter((x) => !x.blocking).length, deferred: Math.max(blocking.length - CLARITY_QUESTIONS, 0), settled: answers.filter((a) => a.settled).length, threads })
}
const clarityPass = async () => {
  const pending = (await step('clarity-pending', {})).keys
  await pipeline(pending, async (key) => {
    try {
      await clarify(key)
    } catch (e) {
      unclear.add(key)
      if (e && e.atCap) capped = true
      log(`${key}: the clarity check ${e && e.atCap ? 'stopped at the agent cap' : 'failed'}: ${(e && e.message) || e}; the story is not delivered in this run`)
    }
  })
}

// ---- 4. deliver ----------------------------------------------------------------
const outcomes = []
// Why the run takes no new story, once a breaker has stopped it: the stories
// in flight finish.
let halted = null
// Notes for the run's report.
const notes = []

// Infrastructure trouble with agents. An agent that returns nothing is asked
// again after each wait in AGAIN_AFTER, with no attempt used, and one that
// returns something ends the row of stories sent back for nothing.
let emptyRow = []
const persist = async (key, who, call) => {
  for (let i = 0; ; i++) {
    const r = await call()
    if (r) {
      emptyRow = []
      return r
    }
    if (i === AGAIN_AFTER.length) return null
    log(`${key}: ${who} returned nothing; asking again in ${AGAIN_AFTER[i]} s, with no attempt used`)
    await step('sleep', { seconds: AGAIN_AFTER[i] }, { key })
  }
}

// An agent that returned nothing after its retries, or nothing usable: the
// story goes back to its waiting state, with no attempt used. Agents that
// returned nothing for EMPTY_STORIES stories in a row are the
// infrastructure's trouble: the run takes no new story, and opens one thread
// for the run.
let emptied = false
const noResult = async (key, what, empty = true) => {
  await step('release', { key })
  if (empty) emptyRow.push(key)
  if (empty && emptyRow.length >= EMPTY_STORIES && !emptied) {
    emptied = true
    const row = emptyRow.join(', ')
    halted = halted || `agents returned nothing for ${row}, in a row`
    try {
      const { thread } = await step('thread-open', { title: `Run ${runId}: agents return nothing`, body: `Agents of run ${runId} returned nothing for ${row}, in a row, each after ${AGAIN_AFTER.length} retries over ${AGAIN_AFTER.reduce((a, s) => a + s, 0)} s; the stories went back to their waiting states with no attempt used.\n\nThe run takes no new story: those in flight finish. Check the Claude Code session that runs it, its limits and its connection, then start a new run.\n` }, { label: 'no-results' })
      notes.push(`The run took no new story once agents returned nothing for ${row}, in a row (thread ${thread}).`)
    } catch (e) {
      log(`The thread about agents that return nothing could not be opened: ${(e && e.message) || e}`)
    }
    log(`Agents returned nothing for ${row}, in a row: the run takes no new story`)
  }
  return { key, outcome: 'no_result', detail: `${what}; the story is released without using an attempt` }
}

// A story stopped at the hard cap goes back to its waiting state, with no
// attempt used; the next run carries on with it.
const stopped = async (key) => {
  capped = true
  try {
    await step('release', { key }, { past: true })
  } catch (e) {
    log(`${key}: the release failed too: ${(e && e.message) || e}`)
  }
  log(`${key}: stopped at ${HARD_CAP} agent calls, and released without using an attempt; the next run carries on`)
  return { key, outcome: 'stopped', detail: `stopped at the agent cap of ${HARD_CAP}` }
}

// A story whose delivery stopped on an error goes back to its waiting state.
// The first story an error stops gets a thread with the error, which blocks
// it: the human hears of it, and no run takes the story again, failing the
// same way, until the thread is answered. The same error stopping a second
// story, its key aside, has a lasting cause, such as a deleted deliver
// branch: the run takes no new story and opens one thread for the run, and
// from then on a story that fails is only released.
const errors = new Map()
let repeated = false
// An error as the breaker compares it: the story's key taken out where it
// stands as a word of its own, so that US-1 is not taken out of US-10.
const causeOf = (message, key) => message.replace(new RegExp(`(^|\\W)${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?!\\w)`, 'g'), '$1<story>')
const failed = async (key, e) => {
  const message = String((e && e.message) || e)
  const cause = causeOf(message, key)
  // Decided before any wait, in the order the chains threw.
  const first = repeated ? null : errors.get(cause)
  if (!repeated && first === undefined) errors.set(cause, key)
  if (first) {
    repeated = true
    halted = `${first} and ${key} stopped on the same error`
  }
  let thread = null
  try {
    await step('release', { key }, { past: true })
  } catch (r) {
    log(`${key}: the release failed too: ${(r && r.message) || r}`)
  }
  if (first) {
    try {
      const runThread = (await step('thread-open', { title: `Run ${runId}: stories stop on the same error`, body: `${first} and ${key} stopped on the same error:\n\n${message}\n\nThe run takes no new story: those in flight finish, and a story that stops on an error after this is only released, with no thread of its own. ${first} is blocked on its own thread, and ${key} is back in its waiting state. Fix the cause, answer the thread of ${first}, then start a new run.\n` }, { label: 'repeated-error', past: true })).thread
      notes.push(`The run took no new story once ${first} and ${key} stopped on the same error (thread ${runThread}).`)
    } catch (t) {
      log(`The thread about the repeated error could not be opened: ${(t && t.message) || t}`)
    }
    log(`${key}: stopped on the same error as ${first}: the run takes no new story`)
  }
  if (first !== undefined) return { key, outcome: 'failed', detail: message }
  try {
    thread = (await step('thread-open', { key, title: `${key}: delivery stopped on an error`, body: `The workflow stopped delivering ${key} on this error:\n\n${message}\n\nThe story is back in its waiting state, and blocked until this thread is answered; answer it once the cause is fixed.\n` }, { past: true })).thread
  } catch (t) {
    log(`${key}: the thread about the error could not be opened: ${(t && t.message) || t}`)
  }
  return { key, outcome: 'failed', detail: message, ...(thread ? { thread } : {}) }
}

const exhaust = async (key, known = null) => {
  const ctx = known || (await step('context', { key }))
  const body = `The story failed ${ctx.attempt} attempts since the human's latest answer. The findings of every attempt:\n\n${byAttempt(ctx.findings)}${ctx.earlier.length ? `\n\nBefore the latest answer:\n\n${byAttempt(ctx.earlier)}` : ''}\n`
  const { thread } = await step('thread-open', { key, title: `${key}: still failing after ${ctx.attempt} attempts`, body })
  log(`${key}: still failing after ${ctx.attempt} attempts; blocked with thread ${thread}`)
  return { key, outcome: 'attempts_exhausted', thread }
}

// The findings of a failed attempt, or of a send-back that uses none (counts
// false), go on the story. After its last attempt the story goes back to the
// backlog with its thread; else it is sent back for its next attempt, and
// stays held where it failed until that starts (deliverStory).
const sendBack = async (key, stage, findings, counts, what) => {
  const { attempt } = await step('finding', { key, stage, findings, counts })
  log(counts ? `${key}: attempt ${attempt} failed ${what}` : `${key}: sent back with no attempt used: ${what}`)
  if (!counts || attempt < MAX_ATTEMPTS) return { rework: true }
  await step('move', { key, to: 'backlog' })
  return exhaust(key)
}

const recordDecisions = async (ctx, decisions) => {
  const known = new Set(ctx.decisions.map((d) => `${d.question}\n${d.choice}`))
  for (const d of decisions || []) {
    if (!d || !d.question || !d.choice || known.has(`${d.question}\n${d.choice}`)) continue
    known.add(`${d.question}\n${d.choice}`)
    await step('decision', { key: ctx.story.key, question: d.question, choice: d.choice, by: 'agent' })
  }
}

// One attempt's implementing: the claim, the worktree, the implementer, run
// again after each question the council settles, the files and the move to
// implemented. Gives the worktree, else the story's outcome.
const implement = async (key, rework) => {
  // A story sent back waited for this slot held where it failed, which kept
  // its files busy: rework moves it to the backlog and claims it again in one
  // helper call, so that no story planned on its files is claimed between
  // the two. Past the hard cap too: a story left where it failed would be
  // judged again with nothing changed.
  if (rework) await step('rework', { key }, { past: true })
  else await step('claim', { key, stage: 'implementing' })
  let ctx = await step('context', { key })
  if (ctx.attempt >= MAX_ATTEMPTS) return exhaust(key, ctx)
  // A worktree an implementer left on another branch: a clean one is back on
  // story/<key>, and the next implementer is told what to bring over.
  const wt = await step('worktree', { key })
  if (wt.reason) log(`${key}: ${wt.reason}`)
  let task = wt.created ? FRESH : CONTINUE
  for (let asked = 0; ; asked++) {
    const attempt = ctx.attempt + 1
    const r = await persist(key, 'the implementer', () => ask('impl', implPrompt(ctx, wt.path, attempt, task, wt.reason), { label: `impl:${key}`, phase: 'Deliver', schema: IMPL_SCHEMA }, MODELS[attempt >= 2 ? 'implRetry' : 'impl']))
    if (!r) return noResult(key, `the implementer returned nothing, ${AGAIN_AFTER.length + 1} times`)
    await recordDecisions(ctx, r.decisions)
    if (r.status === 'needs_decision') {
      const answer = await question(ctx, r, 'the implementer', wt.path, 'impl-q', asked)
      if (!answer.settled) return { key, outcome: 'needs_decision', thread: answer.thread }
      ctx = await step('context', { key })
      task = RESUME
      continue
    }
    if (r.status !== 'ready') {
      const { thread } = await step('thread-open', { key, title: `${key}: the implementer is blocked`, body: `${r.summary}\n` })
      return { key, outcome: 'blocked', thread }
    }
    await step('files', { key })
    await step('move', { key, to: 'implemented' })
    return { path: wt.path }
  }
}

// The reviewer, run again after each question the council settles. Every
// criterion gets a verdict: a review without one for each is asked again, and
// after REVIEW_RETRIES the story is released, since a criterion left out is
// not a criterion met. The script decides the review from those verdicts and
// the defects, not from the reviewer's verdict, which it reads only for a
// question. A test obligation is never judged: the reviewer's obligation
// true counts for no other criterion, and a defect that names only an
// obligation is not the implementer's. A pass needs every other criterion
// met and no other defect; anything else gives the findings, one per
// criterion not met and one per defect.
const review = async (key, wt) => {
  let asked = 0
  let malformed = 0
  for (;;) {
    const ctx = await step('context', { key })
    const n = ctx.story.criteria.length
    const v = await persist(key, 'the reviewer', () => ask('review', reviewPrompt(ctx, wt), { label: `review:${key}`, phase: 'Deliver', schema: reviewSchema(n) }))
    if (!v) return noResult(key, `the reviewer returned nothing, ${AGAIN_AFTER.length + 1} times`)
    if (v.verdict === 'needs_decision') {
      const answer = await question(ctx, v, 'the reviewer', wt, 'review-q', asked++)
      if (!answer.settled) return { key, outcome: 'needs_decision', thread: answer.thread }
      continue
    }
    const judged = new Map()
    for (const c of v.criteria || []) if (c && Number.isInteger(c.index) && c.index >= 0 && c.index < n && !judged.has(c.index)) judged.set(c.index, c)
    const missing = ctx.story.criteria.map((c, i) => i).filter((i) => !judged.has(i))
    if (missing.length) {
      if (malformed++ < REVIEW_RETRIES) {
        log(`${key}: the review gave no verdict on criteria ${missing.join(', ')}; asking again`)
        continue
      }
      return noResult(key, `the reviewer gave no verdict on criteria ${missing.join(', ')}, ${REVIEW_RETRIES + 1} times`, false)
    }
    const owed = obligationsOf(ctx)
    const owedText = new Set(ctx.story.criteria.filter((c, i) => owed.has(i)).map((c) => c.text.trim()))
    const unmet = ctx.story.criteria.map((c, i) => ({ c, verdict: judged.get(i), i })).filter((x) => !owed.has(x.i) && x.verdict.met !== true)
    const defects = (v.defects || []).filter((d) => d && d.text && !owedText.has(String(d.criterion || '').trim()))
    if (!unmet.length && !defects.length) return { pass: true }
    return {
      findings: [
        ...unmet.map(({ c, verdict }) => ({ text: `Criterion not met: "${c.text}". ${verdict.evidence}`, files: [] })),
        ...defects.map((d) => ({ text: d.criterion ? `${d.text} (criterion: "${d.criterion}")` : d.text, files: d.files || [] })),
      ],
    }
  }
}

// One attempt's review: the claim, the reviewer, and the move to reviewed,
// or the findings. Gives reviewed, rework when the story is sent back for the
// next attempt, or the story's outcome.
const assess = async (key, wt) => {
  await step('claim', { key, stage: 'review' })
  let path = wt
  if (!path) {
    const found = await step('worktree', { key })
    if (found.reason) log(`${key}: ${found.reason}`)
    path = found.path
  }
  const verdict = await review(key, path)
  if (verdict.outcome) return verdict
  if (verdict.pass) {
    await step('move', { key, to: 'reviewed' })
    return { reviewed: true }
  }
  return sendBack(key, 'review', verdict.findings, true, `its review with ${verdict.findings.length} finding${verdict.findings.length === 1 ? '' : 's'}`)
}

// An implementer's slot covers only the implementing, so that implementers
// keep working while stories wait for a review or the lane, and a story that
// comes back for another attempt takes a new one. Reviews have a limit of
// their own, and the lane its lock.
const implSlots = semaphore(implementers)
const reviewSlots = semaphore(reviewers)
// A task in a slot: one the caller holds already, or the next one free.
const inSlot = async (slots, held, task) => {
  if (!held) await slots.acquire()
  try { return await task() } finally { slots.release() }
}

// A story from where it waits, backlog, implemented or reviewed, through its
// attempts until it lands or is blocked on a thread. `held`: the deliver loop
// has taken an implementer's slot for its first implementing. A reviewed
// story goes to the lane out of its review slot, and waits for its turn there.
const deliverStory = async (key, from, held) => {
  let at = from
  let rework = false
  let wt = null
  for (;;) {
    if (at === 'backlog') {
      const built = await inSlot(implSlots, held, () => implement(key, rework))
      held = false
      if (built.outcome) return built
      wt = built.path
      at = 'implemented'
    }
    if (at === 'implemented') {
      const assessed = await reviewSlots.run(() => assess(key, wt))
      if (assessed.outcome) return assessed
      at = assessed.rework ? 'rework' : 'reviewed'
    }
    if (at === 'reviewed') {
      const tested = await inLane(() => laneStory(key))
      if (tested.outcome) return tested
    }
    // Sent back, the story stays held where it failed, which keeps its files
    // busy, until an implementer's slot is free: rework goes ahead of new
    // claims, and only then does the story go back to the backlog. Unclaimed
    // there while it waited, another story planned on the same files was
    // claimed beside it.
    await implSlots.acquire(true)
    held = true
    rework = true
    at = 'backlog'
  }
}

// Each free implementer's slot goes to the first story the board lists as
// pickable that this run has not taken: listed again for each claim, since a
// claimed story's planned files keep others off the pickable list. A story
// listed as implemented starts at its review, and one listed as reviewed at
// the lane, unless the lane has stopped. When nothing new is listed, the
// board is listed again once a story in flight is done; the run goes on until
// nothing new is listed and nothing is in flight. A new story is taken only
// while the agent cap leaves room for it beside those in flight, up to
// storyLimit, and until a breaker stops the run.
const taken = new Set()
const running = new Set()
const launch = (key, from, held) => {
  taken.add(key)
  const chain = deliverStory(key, from, held)
    .catch((e) => (e && e.atCap ? stopped(key) : failed(key, e)))
    .then((o) => {
      outcomes.push(o)
      log(`${o.key}: ${o.outcome}${o.detail ? `: ${o.detail}` : ''}${o.thread ? ` (thread ${o.thread})` : ''}`)
    })
    .finally(() => running.delete(chain))
  running.add(chain)
}
// Whether the run may take one more story; says why not, once.
const mayTake = () => {
  if (halted) {
    log(`The run takes no new story: ${halted}`)
    return false
  }
  if (taken.size >= storyLimit) {
    log(`The run has taken ${taken.size} of its storyLimit of ${storyLimit}: it takes no new story`)
    return false
  }
  if (!room(STORY_CALLS, running.size)) {
    capped = true
    log(`The run has made ${agentCalls} agent calls, with ${running.size} stories in flight: it takes no new story, and ends once those are done; the next run carries on`)
    return false
  }
  return true
}
const deliverAll = async () => {
  let taking = true
  try {
    while (taking) {
      await implSlots.acquire()
      let held = true
      try {
        if (!mayTake()) break
        const board = await step('list', {})
        const fresh = (key) => !taken.has(key) && !unclear.has(key)
        for (const s of board.stories.filter((x) => (x.status === 'implemented' || (x.status === 'reviewed' && !laneStopped)) && !x.blocked && !x.holder && fresh(x.key))) {
          if (!(taking = mayTake())) break
          launch(s.key, s.status, false)
        }
        const next = board.pickable.find(fresh)
        if (taking && next && (taking = mayTake())) {
          launch(next, 'backlog', true)
          held = false
          continue
        }
      } finally {
        if (held) implSlots.release()
      }
      if (!running.size) break
      if (taking) await Promise.race(running)
    }
  } finally {
    // The stories in flight finish, also when the board could not be listed.
    await Promise.all(running)
  }
}

// ---- 5. the lane ---------------------------------------------------------------
// One story at a time is tested in the lane worktree, and lands on
// deliver/<slug>. The lane's lock is a promise chain: a story's turn starts
// once the turn before it has ended, however that ended.
let laneTurn = Promise.resolve()
const inLane = (task) => {
  const turn = laneTurn.then(task)
  laneTurn = turn.catch(() => {})
  return turn
}
// Why the lane stopped, once it has: it stays stopped for the rest of the run.
// Implementers and reviewers finish what they hold, and a story that reaches
// the lane after it waits in reviewed for the next run.
let laneStopped = null
const LANE_CHECKS = ['fast', 'stack_up', 'stack']

// A command run as a job in the lane worktree, to its end: one courier starts
// it, and each one after waits for it within its Bash call. A wait that
// throws, on a courier that failed or at the hard cap, stops the job first,
// past the cap: the job would run on beside the lane's next pass.
const job = async (key, name, cmd, past = false) => {
  const { id } = await step('job-start', { name, key, cmd, cwd: laneDir }, { past })
  try {
    for (;;) {
      const w = await step('job-wait', { id, maxSeconds: JOB_WAIT }, { key, past })
      if (w.done) return { ok: w.result.ok === true, timedOut: w.result.timedOut === true, tail: String(w.tail || '').trim() }
    }
  } catch (e) {
    try {
      await step('job-kill', { id }, { key, past: true })
    } catch (k) {
      log(`${key}: the job ${id} could not be stopped: ${(k && k.message) || k}`)
    }
    throw e
  }
}
const ended = (r) => `${r.timedOut ? 'ran past its limit' : 'failed'}. The end of its output:\n\`\`\`\n${r.tail}\n\`\`\``

// Trouble that is not the story's stops the lane for the rest of the run,
// with one thread for the run, and the story goes back to reviewed with no
// attempt used.
const stopLane = async (key, why, more = '') => {
  laneStopped = why
  // A release that fails leaves the story held in testing, which the next
  // run's start releases: the lane stops all the same.
  try {
    await step('release', { key })
  } catch (e) {
    notes.push(`${key} could not be released when the lane stopped, and stays in testing until the next run's start releases it: ${(e && e.message) || e}`)
  }
  let thread = null
  try {
    thread = (await step('thread-open', { title: `Run ${runId}: the lane stopped`, body: `The lane stopped for the rest of run ${runId}, on ${key}: ${why}.${more ? `\n\n${more}` : ''}\n\nThis is not the story's trouble: ${key} is back in reviewed with no attempt used, and a story that reaches the lane after it waits in reviewed for the next run, while implementers and reviewers finish what they hold. Fix the cause, then start a new run.\n` }, { label: 'lane-stop' })).thread
  } catch (e) {
    log(`The thread about the lane could not be opened: ${(e && e.message) || e}`)
  }
  notes.push(`The lane stopped on ${key}: ${why}${thread ? ` (thread ${thread})` : ''}.`)
  log(`${key}: the lane stopped for the rest of the run: ${why}`)
  return { key, outcome: 'lane_stopped', detail: why }
}

// Rebase conflicts that sent each story back in this run.
const conflicts = new Map()

// A story's turn in the lane: lane-prepare rebases it onto the tip and checks
// it out, detached, in the lane worktree; its checks run there as jobs, and a
// failed one once more on the base, after stack_reset for a stack check; the
// verifier runs it; the test author writes its tests; lane-clean, so that the
// author's check runs on exactly what lands; that check, and lane-clean again
// for what it left; lane-land. A failure sends the story back for the next
// attempt, as a failed review does, and puts the lane back at the tip, with
// the stack reset and rebuilt there when it holds the story's build. The
// test author's own trouble costs the verified story nothing: a red check
// gets the author once more, with its output, and tests whose check is still
// red, or that lane-land refuses for the state they left the lane in, are
// dropped, and the story lands without them.
const laneStory = async (key) => {
  if (laneStopped) return { key, outcome: 'reviewed', detail: 'the lane stopped earlier in this run, so the story waits in reviewed for the next run' }
  await step('claim', { key, stage: 'testing' })
  const prep = await step('lane-prepare', { key })
  // An outcome but ready leaves the lane as it was.
  if (prep.outcome === 'infra') return stopLane(key, `lane-prepare found trouble that is not the story's: ${prep.reason}`)
  if (prep.outcome === 'conflict') {
    const n = (conflicts.get(key) || 0) + 1
    conflicts.set(key, n)
    return sendBack(key, 'testing', [{ text: `rebase conflict in ${prep.files.join(', ')}: story/${key} no longer rebases onto ${branch}; rebase it onto ${branch} in its worktree and resolve the conflicts`, files: prep.files }], n > CONFLICTS, `a rebase conflict in ${prep.files.join(', ')}${n > CONFLICTS ? `, conflict ${n} of this run, past the ${CONFLICTS} that use no attempt` : ''}`)
  }
  if (prep.outcome === 'missing') return sendBack(key, 'testing', [{ text: `${prep.reason}: nothing to land on ${branch}; start again from ${branch}`, files: [] }], false, prep.reason)
  if (prep.outcome !== 'ready') return sendBack(key, 'testing', [{ text: prep.reason, files: prep.files || [] }], true, `in the lane: ${prep.reason}`)
  const base = prep.base
  // Whether the stack holds the story's build: from its stack_up on, until a
  // stack_up on the base or the rebuild.
  let stack = false
  let reset = false
  // stack_reset, when the project sets one, clears what a story left in the
  // stack, such as a migration in its database, before the stack is built
  // from a tree without the story, whose stack_up would fail on it.
  const toTip = async (past = false) => {
    if (reset) return
    await step('lane-reset', {}, { key, past })
    reset = true
    if (!stack) return
    stack = false
    if (checks.stack_reset) {
      const cleared = await job(key, 'stack_reset', checks.stack_reset, past)
      if (!cleared.ok) log(`${key}: the stack did not reset: \`${checks.stack_reset}\` ${ended(cleared)}`)
    }
    const r = await job(key, 'rebuild', checks.stack_up, past)
    log(r.ok ? `${key}: the stack is rebuilt from ${branch}` : `${key}: the stack did not rebuild from ${branch}: \`${checks.stack_up}\` ${ended(r)}`)
  }
  const fail = async (findings, what) => {
    await toTip()
    return sendBack(key, 'testing', findings, true, what)
  }
  try {
    for (const name of LANE_CHECKS.filter((n) => checks[n])) {
      if (name === 'stack_up') stack = true
      const run = await job(key, name, checks[name])
      if (run.ok) continue
      // Once more on the base, the tip without the story: a stack check after
      // stack_reset, and the stack check after a stack_up there, since until
      // then the stack holds the story's build and what it left.
      await step('lane-base', {}, { key })
      let which = null
      let again = null
      for (const n of [...(name !== 'fast' && checks.stack_reset ? ['stack_reset'] : []), ...(name === 'stack' && checks.stack_up ? ['stack_up', 'stack'] : [name])]) {
        which = n
        if (n === 'stack_up') stack = false
        again = await job(key, n, checks[n])
        if (!again.ok) break
      }
      if (!again.ok) {
        await toTip()
        const state = which === 'fast' ? '' : '\n\nIf a story left state on the stack, make stack_reset (or stack_up) reset it: a migration of a story the lane sent back, for one, stays in the stack\'s database, and fails every stack_up of a tree without it.'
        return stopLane(key, `the ${name} check failed on it, and the ${which} check on ${branch} without it`, `On ${branch} at ${base.slice(0, 7)}, \`${checks[which]}\` ${ended(again)}${state}`)
      }
      return fail([{ text: `The ${name} check (\`${checks[name]}\`) ${ended(run)}\nIt passes on ${branch} without the story.`, files: [] }], `its ${name} check, which passes on ${branch} without it`)
    }

    const ctx = await step('context', { key })
    const v = await persist(key, 'the verifier', () => ask('verify', verifyPrompt(ctx), { label: `verify:${key}`, phase: 'Lane', schema: VERIFY_SCHEMA }))
    if (!v) {
      await toTip()
      return noResult(key, `the verifier returned nothing, ${AGAIN_AFTER.length + 1} times`)
    }
    // A test obligation is the test author's, whatever the verifier says of it.
    const owed = obligationsOf(ctx)
    const ran = new Map()
    for (const c of v.criteria || []) if (c && Number.isInteger(c.index) && !ran.has(c.index)) ran.set(c.index, c)
    const unmet = ctx.story.criteria.flatMap((c, i) => (owed.has(i) || (ran.has(i) && ran.get(i).ok === true) ? [] : [{ text: `Criterion not met when run: "${c.text}". ${ran.has(i) ? ran.get(i).evidence : 'The verifier did not exercise it.'}`, files: [] }]))
    if (unmet.length) return fail(unmet, `its verification with ${unmet.length} criteri${unmet.length === 1 ? 'on' : 'a'} not met`)

    const confirmed = ctx.story.criteria.filter((c, i) => !owed.has(i)).map((c) => c.text)
    let tctx = ctx
    let asked = 0
    // The test author, run again after each question the council settles:
    // its result, or how the story's turn ends.
    const author = async (task) => {
      for (;;) {
        const r = await persist(key, 'the test author', () => ask('tests', authorPrompt(tctx, base, confirmed, task), { label: `tests:${key}`, phase: 'Lane', schema: TESTS_SCHEMA }))
        if (!r) {
          await toTip()
          return { end: await noResult(key, `the test author returned nothing, ${AGAIN_AFTER.length + 1} times`) }
        }
        if (r.status === 'needs_decision') {
          const answer = await question(tctx, r, 'the test author', laneDir, 'tests-q', asked++)
          if (!answer.settled) {
            await toTip()
            return { end: { key, outcome: 'needs_decision', thread: answer.thread } }
          }
          tctx = await step('context', { key })
          task = RESUME_TESTS
          continue
        }
        if (r.status === 'found_defect') {
          const d = r.defect || {}
          return { end: await fail([{ text: `The test author's test ${d.test || '(unnamed)'} found a defect. Observed: ${d.observed || '(not given)'}. Expected: ${d.expected || '(not given)'}`, files: [] }], 'its tests, which found a defect') }
        }
        if (r.status !== 'done') {
          const { thread } = await step('thread-open', { key, title: `${key}: the test author is blocked`, body: `${r.blockers || r.status}\n` })
          await toTip()
          return { end: { key, outcome: 'blocked', thread } }
        }
        return { tests: r }
      }
    }
    // lane-clean, so that the author's check runs on exactly what lands; that
    // check; and lane-clean again for what the check left, such as a report.
    // Gives the check and its run when it is red. An author asked once more
    // that gives no check is held to the check it gave before.
    const checkTests = async (tests, before = '') => {
      const cleaned = await step('lane-clean', {}, { key })
      if (cleaned.count) log(`${key}: lane-clean threw away ${cleaned.count} file${cleaned.count === 1 ? '' : 's'} the test author left uncommitted: ${cleaned.discarded.join(', ')}`)
      const check = String(tests.check || before).trim()
      if (!check) return null
      const run = await job(key, 'tests', check)
      const after = await step('lane-clean', {}, { key })
      if (after.count) log(`${key}: lane-clean threw away ${after.count} file${after.count === 1 ? '' : 's'} the test author's check left: ${after.discarded.join(', ')}`)
      return run.ok ? null : { check, run }
    }
    let written = await author(WRITE_TESTS)
    if (written.end) return written.end
    // Why the story lands without its tests, once they are dropped.
    let without = null
    let red = await checkTests(written.tests)
    if (red) {
      log(`${key}: the test author's check \`${red.check}\` ${red.run.timedOut ? 'ran past its limit' : 'failed'}; the author is asked once more, with its output`)
      written = await author(fixTests(red.check, red.run))
      if (written.end) return written.end
      red = await checkTests(written.tests, red.check)
      if (red) {
        without = `the test author's check \`${red.check}\` ${red.run.timedOut ? 'ran past its limit' : 'failed'} again, after the author was asked once more`
        await step('lane-candidate', { candidate: prep.candidate }, { key })
      }
    }
    let landed = await step('lane-land', { key, base })
    if (!landed.landed && landed.inLane && !without) {
      without = `lane-land refused the lane the test author left: ${landed.reason}`
      await step('lane-candidate', { candidate: prep.candidate }, { key })
      landed = await step('lane-land', { key, base })
    }
    if (!landed.landed) return fail([{ text: `It did not land: ${landed.reason}`, files: [] }], `its landing: ${landed.reason}`)
    tip = landed.final
    if (without) {
      notes.push(`${key} landed without regression tests: ${without}.`)
      log(`${key}: landed without regression tests: ${without}`)
    } else {
      for (const a of written.tests.added || []) if (a && a.test) testsAdded.push(`[${key}] ${a.test}${a.file ? ` (${a.file})` : ''}`)
    }
    log(`${key}: landed on ${branch} at ${tip.slice(0, 7)}`)
    return { key, outcome: 'done' }
  } catch (e) {
    // The lane goes back to the tip whatever stopped the story, past the agent
    // cap too; the error goes on to the story's chain.
    try {
      await toTip(true)
    } catch (r) {
      log(`${key}: the lane could not be put back at the tip: ${(r && r.message) || r}`)
    }
    throw e
  }
}

// ---- 6. report -----------------------------------------------------------------
// The journeys check of the stories this run landed, then finish, which posts
// the report on the Run thread and gives the lock up. The run does not wait
// for answers: the next run's unblock pass takes them.
const journeys = async () => {
  const landed = outcomes.filter((o) => o.outcome === 'done').map((o) => o.key)
  if (!landed.length) return ['no story landed in this run, so no journeys were run']
  // The journeys need the lane at the tip; without it the run still finishes,
  // and its report says why they did not run.
  try {
    await step('lane-reset', {})
  } catch (e) {
    if (e && e.atCap) return [`not run: the run reached its agent cap of ${HARD_CAP}`]
    log(`The lane worktree could not be put back at the tip for the journeys check: ${(e && e.message) || e}`)
    return [`not run: the lane worktree could not be put back at the tip of ${branch}: ${(e && e.message) || e}`]
  }
  try {
    const r = await ask('final', finalPrompt(landed), { label: 'final', phase: 'Report', schema: FINAL_SCHEMA })
    if (!r) return ['the journeys check returned nothing']
    return [`${r.healthy ? 'healthy' : 'NOT healthy'}: ${r.report || ''}`.trim(), ...(r.journeys || []).map(String)]
  } catch (e) {
    if (e && e.atCap) return [`not run: the run reached its agent cap of ${HARD_CAP}`]
    throw e
  }
}
// The stories blocked when the run ends, with their threads.
const blockedNow = async () => {
  const board = await step('list', {}, { past: true })
  const blocked = []
  for (const s of board.stories.filter((x) => x.blocked && x.status !== 'done' && x.status !== 'cancelled')) {
    blocked.push({ key: s.key, threads: agentCalls < HARD_CAP ? (await step('context', { key: s.key })).threads : [] })
  }
  return blocked
}
let finished = null
const finish = async (journeyLines) => {
  if (current !== 'Report') enter('Report')
  let blocked = []
  try {
    blocked = await blockedNow()
  } catch (e) {
    notes.push(`The blocked stories could not be listed: ${(e && e.message) || e}`)
  }
  if (capped) notes.push(`The run stopped taking stories at its agent cap, after ${agentCalls} agent calls; the next run carries on.`)
  if (taken.size >= storyLimit) notes.push(`The run took its storyLimit of ${storyLimit} stories.`)
  const report = {
    outcomes: outcomes.map((o) => ({ key: o.key, outcome: o.outcome, ...(o.detail ? { detail: o.detail } : {}) })),
    council: tally,
    blocked,
    ...(journeyLines ? { journeys: journeyLines } : {}),
    notes,
  }
  finished = await step('finish', { report }, { past: true })
  log(`The report of ${runId} is on the Run thread ${finished.thread}, and in ${finished.report}`)
}

// ---- the run -------------------------------------------------------------------
// Every step after start runs here, and finish runs whenever start succeeded,
// after a throw too, so that the lock is given up. A finish that fails after
// a throw is only logged: the run ends with the throw.
let journeyLines = null
let threw = true
try {
  if (A.checks && typeof A.checks === 'object') checks = (await step('checks-set', { checks: A.checks })).checks
  enter('Unblock')
  await unblock()
  enter('Clarity')
  await clarityPass()
  enter('Deliver')
  await deliverAll()
  enter('Report')
  journeyLines = await journeys()
  threw = false
} catch (e) {
  notes.push(`The run stopped on an error: ${(e && e.message) || e}`)
  throw e
} finally {
  try {
    await finish(journeyLines)
  } catch (e) {
    if (!threw) throw e
    log(`The run could not finish: ${(e && e.message) || e}`)
  }
}

return {
  run: runId,
  feature,
  root,
  branch,
  thread: started.thread,
  checks,
  released: started.released,
  unblocked: unblocked.unblocked,
  clarity,
  outcomes,
  council: tally,
  agents: agentCalls,
  capped,
  halted,
  lane: laneStopped,
  notes,
  storyLimit: storyLimit === Infinity ? null : storyLimit,
  // The landed state: the stories this run landed, and the tip of the branch.
  landed: outcomes.filter((o) => o.outcome === 'done').map((o) => o.key),
  tip,
  report: finished.report,
  posted: finished.posted,
}
