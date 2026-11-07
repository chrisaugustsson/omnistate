export const meta = {
  name: 'omni-deliver',
  description: 'Delivers a feature\'s stories from the OmniState board: clarity checks with a council in front of every question for the human, then implement and review in story worktrees, at most `implementers` stories implementing and `reviewers` in review at once. Every git and omni command runs in the host helper, reached through courier agents',
  whenToUse: 'Run with the Workflow tool from the main worktree of a project whose folder is linked to its OmniState project (omni link, or OMNI_PROJECT), with args {feature: "<name>"} (optional: implementers (3), reviewers (3), storyLimit, force, checks, models, agentTypes, helper). The host helper is ~/.claude/omni/deliver-step.mjs, as make install-agent-tools links it; args.helper names another path. One run per checkout: the helper\'s .omni/run.lock refuses a second live run, and force takes over a lock whose run the human knows is dead. args.checks ({fast, stack_up, stack, seed, lint}) sets checks on top of omni checks. args.models overrides the model of a role (clarity, council, impl, implRetry, review, verify, tests, final, courier), and args.agentTypes sets the agent type of a role, which has none by default. Stories land on deliver/<feature slug>, never on the checkout\'s branch. A run takes a new story only while its agent calls leave room under 900 for the stories in flight, at 60 each, and stops a story at 960, released with no attempt used; the next run carries on. A story whose delivery stops on an error is released and blocked on a thread with the error. Built so far: start, unblock, clarity, implement and review; a reviewed story waits in reviewed for the lane, which is not built yet. Never pushes.',
  phases: [
    { title: 'Start', detail: 'the helper takes the lock, reconciles claims and the lane worktree, sets deliver/<slug> and reads the checks' },
    { title: 'Unblock', detail: 'a story whose threads all have a conclusion gets each one as a human decision and is unblocked, and its attempts count from there' },
    { title: 'Clarity', detail: 'one check per backlog story whose text, criteria, linked nodes or decisions changed: a criterion that names a test becomes a test obligation, a minor gap takes its recommendation, the 3 blocking gaps of most impact go to the council and the rest take their recommendation, deferred by the cap; what the council cannot settle becomes a thread that blocks the story' },
    { title: 'Deliver', detail: 'at most `implementers` stories implementing at once, each slot free again once its story is implemented: claim, worktree, implementer (on implRetry from attempt 2, with every finding by attempt), files; then at most `reviewers` in review, with a verdict per criterion and every defect; a failed review sends the story back for the next attempt, in a new implementer slot, and 3 failed attempts open a thread; a question goes to the council first' },
  ],
}

// omni-deliver as a native workflow (tasks/omni-deliver-native-design.md).
// The script holds the flow: which story goes where, attempts, the council.
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
// one story has used 58. A story is claimed, and a clarity check started,
// only while the calls made and those set aside for the work in flight
// leave room under AGENT_CAP; the next run carries on. Work that reaches
// HARD_CAP all the same stops there: its story is released with no attempt
// used.
const AGENT_CAP = 900
const HARD_CAP = 960
const STORY_CALLS = 60
const CHECK_CALLS = 20
const COUNCILS = 2
const COURIER_RETRIES = 3
// A review that leaves a criterion without a verdict is asked again this
// many times.
const REVIEW_RETRIES = 2

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

// At most n holders at once; the others wait their turn, first come first served.
const semaphore = (n) => {
  let free = n
  const queue = []
  const acquire = () => {
    if (free > 0) {
      free--
      return Promise.resolve()
    }
    return new Promise((resolve) => queue.push(resolve))
  }
  const release = () => {
    const next = queue.shift()
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
// lock, a worktree on the wrong branch) is not tried again.
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
// author owns. Clarity records it as this decision; a story implemented
// before clarity ran has none, so the reviewer also tells them apart.
const testQuestion = (criterion) => `Is "${criterion}" behaviour or a test?`
const TEST_OBLIGATION = 'treated as a test obligation: the lane\'s test author owns it, and the verifier skips it'
const decidedObligations = (ctx) => new Set(ctx.story.criteria.flatMap((c, i) => (ctx.decisions.some((d) => d.question === testQuestion(c.text) && d.choice === TEST_OBLIGATION) ? [i] : [])))
const numberedCriteria = (ctx) => {
  const decided = decidedObligations(ctx)
  return ctx.story.criteria.map((c, i) => `${i}: ${c.text}${c.human_check ? ' [human check]' : ''}${decided.has(i) ? ' [test obligation]' : ''}`).join('\n') || '(none)'
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
const CONVENTIONS = `
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
- Work on the CURRENT branch. Do not push, create branches or amend.
`
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
const implPrompt = (ctx, wt, attempt, task) => `You implement ONE story of the feature "${feature}" in its own git worktree.

## Your worktree
${wt} (branch story/${ctx.story.key}, cut from ${branch}). Work only there: cd into it first, and run every command there.

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
          obligation: { type: 'boolean', description: 'true for a test obligation, a criterion marked [test obligation] or one that names a test, which you do not judge' },
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
- A new test, test file, test helper, fixture or snapshot in the diff is a defect: the implementer writes no tests, the lane's test author does. An existing test updated to a decision the story names is fine.
- A criterion marked [test obligation], or any other that names a test (a test, test file, suite or case to write or to pass), is a test obligation: the lane's test author owns it, and the review never judges it. Give it obligation true.
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
for (const line of started.log || []) log(line)
log(`Run ${runId} on ${feature}: stories land on ${branch}, now at ${String(started.tip).slice(0, 7)}`)
let checks = started.checks
if (A.checks && typeof A.checks === 'object') checks = (await step('checks-set', { checks: A.checks })).checks

// ---- 2. unblock --------------------------------------------------------------
// A thread's conclusion is the human's answer, and restarts the story's attempts.
enter('Unblock')
const unblocked = await step('unblock-pass', {})
for (const line of unblocked.log || []) log(line)
if (unblocked.unblocked.length) log(`Unblocked ${unblocked.unblocked.join(', ')}`)

// ---- 3. clarity --------------------------------------------------------------
enter('Clarity')
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

// ---- 4. deliver ----------------------------------------------------------------
enter('Deliver')
const outcomes = []

// An agent that returned nothing, or nothing usable: the story goes back to
// its waiting state, with no attempt used. The second half of build step 3
// replaces this with the retries after a wait.
const noResult = async (key, what) => {
  await step('release', { key })
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

// A story whose delivery stopped on an error goes back to its waiting state
// and gets a thread with the error, which blocks it: the human hears of it,
// and no run takes the story again, failing the same way, until the thread
// is answered.
const failed = async (key, e) => {
  const message = String((e && e.message) || e)
  let thread = null
  try {
    await step('release', { key }, { past: true })
  } catch (r) {
    log(`${key}: the release failed too: ${(r && r.message) || r}`)
  }
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
const implement = async (key) => {
  await step('claim', { key, stage: 'implementing' })
  let ctx = await step('context', { key })
  if (ctx.attempt >= MAX_ATTEMPTS) return exhaust(key, ctx)
  const wt = await step('worktree', { key })
  let task = wt.created ? FRESH : CONTINUE
  for (let asked = 0; ; asked++) {
    const attempt = ctx.attempt + 1
    const r = await ask('impl', implPrompt(ctx, wt.path, attempt, task), { label: `impl:${key}`, phase: 'Deliver', schema: IMPL_SCHEMA }, MODELS[attempt >= 2 ? 'implRetry' : 'impl'])
    if (!r) return noResult(key, 'the implementer returned nothing')
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
// not a criterion met. A test obligation is never judged. A pass needs the
// verdict pass, every other criterion met and no defect; anything else gives
// the findings, one per criterion not met and one per defect.
const review = async (key, wt) => {
  let asked = 0
  let malformed = 0
  for (;;) {
    const ctx = await step('context', { key })
    const n = ctx.story.criteria.length
    const v = await ask('review', reviewPrompt(ctx, wt), { label: `review:${key}`, phase: 'Deliver', schema: reviewSchema(n) })
    if (!v) return noResult(key, 'the reviewer returned nothing')
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
      return noResult(key, `the reviewer gave no verdict on criteria ${missing.join(', ')}, ${REVIEW_RETRIES + 1} times`)
    }
    const decided = decidedObligations(ctx)
    const unmet = ctx.story.criteria.map((c, i) => ({ c, verdict: judged.get(i), i })).filter((x) => !decided.has(x.i) && x.verdict.obligation !== true && x.verdict.met !== true)
    const defects = (v.defects || []).filter((d) => d && d.text)
    if (v.verdict === 'pass' && !unmet.length && !defects.length) return { pass: true }
    const findings = [
      ...unmet.map(({ c, verdict }) => ({ text: `Criterion not met: "${c.text}". ${verdict.evidence}`, files: [] })),
      ...defects.map((d) => ({ text: d.criterion ? `${d.text} (criterion: "${d.criterion}")` : d.text, files: d.files || [] })),
    ]
    return { findings: findings.length ? findings : [{ text: 'The reviewer failed the story without naming a criterion not met or a defect', files: [] }] }
  }
}

// One attempt's review: the claim, the reviewer, and the move to reviewed,
// or the findings and the move back to the backlog. Gives the story's
// outcome, or null when it goes back for the next attempt.
const assess = async (key, wt) => {
  await step('claim', { key, stage: 'review' })
  const verdict = await review(key, wt || (await step('worktree', { key })).path)
  if (verdict.outcome) return verdict
  if (verdict.pass) {
    await step('move', { key, to: 'reviewed' })
    return { key, outcome: 'reviewed' }
  }
  const { attempt } = await step('finding', { key, stage: 'review', findings: verdict.findings, counts: true })
  log(`${key}: attempt ${attempt} failed its review with ${verdict.findings.length} finding${verdict.findings.length === 1 ? '' : 's'}`)
  await step('move', { key, to: 'backlog' })
  return attempt >= MAX_ATTEMPTS ? exhaust(key) : null
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

// A story from where it waits, backlog or implemented, through its attempts
// until it is reviewed or blocked on a thread. `held`: the deliver loop has
// taken an implementer's slot for its first implementing.
const deliverStory = async (key, from, held) => {
  let implementing = from === 'backlog'
  let wt = null
  for (;;) {
    if (implementing) {
      const built = await inSlot(implSlots, held, () => implement(key))
      held = false
      if (built.outcome) return built
      wt = built.path
    }
    const assessed = await reviewSlots.run(() => assess(key, wt))
    if (!assessed) {
      implementing = true
      continue
    }
    // ---- hand-off: the lane ------------------------------------------------
    // The second half of build step 3 takes a reviewed story into the lane
    // here, out of its review slot and under the lane lock: lane-prepare, the
    // checks as jobs with the rerun on the base, the verifier, the test
    // author, lane-clean, the test author's check and lane-land. A lane
    // failure sends the story back for the next attempt, as a failed review
    // does; so do lane-prepare's dirty and wrong_branch, with the reason as a
    // finding, while a conflict uses no attempt and infra is not the
    // story's. Until then a reviewed story waits in reviewed, as do the
    // stories that were reviewed when the run started.
    return assessed
  }
}

// Each free implementer's slot goes to the first story the board lists as
// pickable that this run has not taken: listed again for each claim, since a
// claimed story's planned files keep others off the pickable list. A story
// listed as implemented and waiting starts at its review. When nothing new
// is listed, the board is listed again once a story in flight is done; the
// run goes on until nothing new is listed and nothing is in flight. A new
// story is taken only while the agent cap leaves room for it beside those in
// flight, and up to storyLimit.
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
let taking = true
while (taking) {
  await implSlots.acquire()
  if (!mayTake()) {
    implSlots.release()
    break
  }
  const board = await step('list', {})
  const fresh = (key) => !taken.has(key) && !unclear.has(key)
  for (const s of board.stories.filter((x) => x.status === 'implemented' && !x.blocked && !x.holder && fresh(x.key))) {
    if (!(taking = mayTake())) break
    launch(s.key, 'implemented', false)
  }
  const next = board.pickable.find(fresh)
  if (taking && next && (taking = mayTake())) {
    launch(next, 'backlog', true)
    continue
  }
  implSlots.release()
  if (!running.size) break
  if (taking) await Promise.race(running)
}
await Promise.all(running)

// ---- hand-off: the rest of the run -------------------------------------------
// The second half of build step 3 goes on from here: failure handling (agents
// that return nothing get retried after a wait; 3 such stories in a row stop
// new claims), the journeys check and finish, which posts the report with the
// council's counts and gives the lock up. The run does not wait for answers:
// the next run's unblock pass takes them.
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
  storyLimit: storyLimit === Infinity ? null : storyLimit,
}
