export const meta = {
  name: 'omni-deliver',
  description: 'Delivers a feature\'s stories from the OmniState board: locks the main worktree, reconciles claims left by dead runs, and clarity-checks the backlog before any worktree exists',
  whenToUse: 'Run from the main worktree of a project with a clean tree, with args {feature: "<name>"} (optional: implementers (3), reviewers (3), storyLimit, models). One run per checkout, whatever the feature: the lock file .omni/run.lock holds the run, and a live pid refuses a second one. Needs the omni CLI on PATH with a project in .omni.toml. Stories come from `omni story`; questions for the human become threads on the feature named after the story. Never pushes.',
  phases: [
    { title: 'Start', detail: 'clean tree, lock, the Run thread' },
    { title: 'Reconcile', detail: 'release claims of other runs, then claim, reset or keep each story by its status' },
    { title: 'Clarity', detail: 'one check per backlog story whose version is new, in parallel: blocking gaps become threads, minor ones agent decisions' },
  ],
}

// ---- args ------------------------------------------------------------------
const A = (args && typeof args === 'object') ? args : {}
if (!A.feature) throw new Error('Pass the feature: args {feature: "<name>"}')
const feature = A.feature
const implementers = A.implementers || 3
const reviewers = A.reviewers || 3
const storyLimit = A.storyLimit || Infinity
// ---- model routing per role ------------------------------------------------
// Same mapping as ralph-spec: Sonnet builds, Opus judges. Override per role with
// args.models, or force one model everywhere with args.model.
const MODELS = { clarity: 'opus', impl: 'sonnet', implRetry: 'opus', review: 'opus', verify: 'opus', tests: 'opus', ...(A.models || {}) }
const modelFor = (role) => A.model || MODELS[role]
const AGENT_TYPES = { clarity: 'verifier', impl: 'implementer', review: 'verifier', verify: 'verifier', tests: 'implementer', ...(A.agentTypes || {}) }
const runAgent = (prompt, { role, ...opts } = {}) =>
  agent(prompt, AGENT_TYPES[role] ? { ...opts, agentType: AGENT_TYPES[role] } : opts)

// ---- git and omni through node:child_process -------------------------------
const cp = await import('node:child_process')
const fs = await import('node:fs')
const path = await import('node:path')

const run = (cmd, argv, opts = {}) => cp.execFileSync(cmd, argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts })
const tryRun = (cmd, argv, opts) => { try { return run(cmd, argv, opts) } catch { return null } }
const root = run('git', ['rev-parse', '--show-toplevel']).trim()
const git = (argv, opts) => run('git', ['-C', root, ...argv], opts)
const repoName = path.basename(root)
const storiesDir = path.join(root, '..', `${repoName}-stories`)
const lockPath = path.join(root, '.omni', 'run.lock')
const clarityPath = path.join(root, '.omni', 'clarity.json')

const pad = (n) => String(n).padStart(2, '0')
const stamp = () => {
  const d = new Date()
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`
}
const alive = (pid) => {
  if (!Number.isInteger(pid) || pid <= 0) return false
  try { process.kill(pid, 0); return true } catch (e) { return e.code === 'EPERM' }
}

// ---- phase 1: clean tree, lock, the Run thread -----------------------------
phase('Start')
if (git(['status', '--porcelain']).trim() !== '') throw new Error('the tree is not clean')
let held = null
try { held = JSON.parse(fs.readFileSync(lockPath, 'utf8')) } catch {}
if (held && alive(held.pid)) throw new Error(`the tree is held by run ${held.run} for feature ${held.feature}`)

const runId = `run-${stamp()}-${process.pid}`
const integration = git(['rev-parse', '--abbrev-ref', 'HEAD']).trim()
fs.mkdirSync(path.dirname(lockPath), { recursive: true })
fs.writeFileSync(lockPath, JSON.stringify({ run: runId, pid: process.pid, feature, started: new Date().toISOString() }) + '\n')

// Every omni call acts as this run's agent, in the feature.
const omni = (argv, opts = {}) => run('omni', [...argv, '--feature', feature], { ...opts, env: { ...process.env, OMNI_RUN: runId, OMNI_ROLE: 'deliver' } })
const omniJson = (argv) => {
  const parsed = JSON.parse(omni([...argv, '--json']))
  return parsed.story || parsed
}
const listStories = () => JSON.parse(omni(['story', 'list', '--json'])).stories
const readStory = (key) => omniJson(['story', 'read', key])
const write = (verb, key, extra = []) => omni(['story', verb, key, '--run', runId, ...extra])

try {
  log(`Run ${runId} on feature ${feature}, integration branch ${integration}`)
  const threadId = omni(['thread', 'create', `Run ${runId}`]).trim()

  // ---- phase 2: reconcile --------------------------------------------------
  phase('Reconcile')
  const reconciled = []
  const worktreeOf = (key) => path.join(storiesDir, key)
  const commitsAhead = (key) => {
    const wt = worktreeOf(key)
    if (!fs.existsSync(wt)) return 0
    const out = tryRun('git', ['-C', wt, 'rev-list', '--count', `${integration}..HEAD`])
    return out === null ? 0 : Number(out.trim()) || 0
  }
  const dropWorktree = (key) => {
    const wt = worktreeOf(key)
    if (fs.existsSync(wt)) {
      if (tryRun('git', ['-C', root, 'worktree', 'remove', '--force', wt]) === null) fs.rmSync(wt, { recursive: true, force: true })
    }
    tryRun('git', ['-C', root, 'worktree', 'prune'])
    tryRun('git', ['-C', root, 'branch', '-D', `story/${key}`])
  }
  for (const story of listStories()) {
    const key = story.key
    if (story.status === 'done' || story.status === 'cancelled') continue
    if (story.holder && story.holder.run !== runId) write('release', key)
    if (story.blocked) {
      if (story.holder) reconciled.push({ key, action: 'released' })
      continue
    }
    if (story.status === 'implementing' || story.status === 'review') {
      if (commitsAhead(key) > 0) {
        write('claim', key, ['--stage', story.status])
        reconciled.push({ key, action: 'claimed', stage: story.status })
      } else {
        dropWorktree(key)
        write('claim', key, ['--stage', story.status])
        write('move', key, ['--to', 'backlog'])
        write('release', key)
        reconciled.push({ key, action: 'backlog' })
      }
    } else if (story.status === 'testing') {
      write('claim', key, ['--stage', 'testing'])
      reconciled.push({ key, action: 'claimed', stage: 'testing' })
    } else if (story.holder) {
      reconciled.push({ key, action: 'released' })
    }
  }

  // ---- phase 3: clarity ----------------------------------------------------
  phase('Clarity')
  const CLARITY_RULE = `
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
  option; it will be built and verified as if the spec said so, and recorded as an
  agent decision for the human to see.
`
  const CLARITY_SCHEMA = {
    type: 'object',
    required: ['gaps'],
    properties: {
      gaps: {
        type: 'array',
        description: 'Criteria in this story that fail the two-implementation test. Empty when every criterion decides the behaviour',
        items: {
          type: 'object',
          required: ['criterion', 'question', 'options', 'recommendation', 'reason', 'blocking'],
          properties: {
            blocking: { type: 'boolean', description: 'true only for a blocking gap per the rule; a minor gap is decided by your recommendation' },
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
  const clarityPrompt = (story) => `You check ONE story of the feature "${feature}" for gaps before any code is written. You read; you write nothing.

## The story record
${JSON.stringify({ key: story.key, title: story.title, text: story.text, criteria: story.criteria, blocked_by: story.blocked_by, files_planned: story.files_planned, decisions: story.decisions }, null, 2)}

## The shared parts
The rule below refers to the feature's Spec, Architecture, DB and UI docs. List them with \`omni doc list --feature ${feature}\` and read each with \`omni doc read <doc>\`. Treat them as the shared parts of every story: a criterion they already settle is not a gap.

## Recorded decisions
The story's "decisions" above are settled. A gap one of them already settles is NOT raised again.
${CLARITY_RULE}
Return raw structured data only.`

  let seen = {}
  try { seen = JSON.parse(fs.readFileSync(clarityPath, 'utf8')) } catch {}
  const toCheck = listStories().filter((s) => s.status === 'backlog' && !s.blocked && seen[s.key] !== s.version)
  const checked = await parallel(toCheck.map((story) => async () => {
    for (let attempt = 0; attempt < 2; attempt++) {
      const r = await runAgent(clarityPrompt(story), { schema: CLARITY_SCHEMA, label: `clarity:${story.key}`, phase: 'Clarity', model: modelFor('clarity'), role: 'clarity' })
      if (r) return { story, gaps: r.gaps || [] }
    }
    return { story, gaps: null }
  }))
  const clarity = []
  for (const result of checked) {
    if (!result || result.gaps === null) {
      log(`Clarity check returned nothing for ${result ? result.story.key : '?'}; it is checked again at the next start`)
      continue
    }
    const { story, gaps } = result
    const key = story.key
    const threads = []
    for (const gap of gaps.filter((g) => g.blocking !== false)) {
      const id = omni(['thread', 'create', `${key}: ${gap.question}`]).trim()
      const summary = `Options and recommendation for ${key}`.slice(0, 200)
      const body = `${gap.question}\n\nCriterion: ${gap.criterion}\n\nOptions:\n${gap.options.map((o, i) => `${i + 1}. ${o}`).join('\n')}\n\nRecommendation: ${gap.recommendation}\n\nWhy: ${gap.reason}\n`
      omni(['post', id, '--summary', summary], { input: body, stdio: ['pipe', 'pipe', 'pipe'] })
      threads.push({ thread: id, question: gap.question })
    }
    if (threads.length) {
      const fresh = readStory(key)
      const links = [...(fresh.links || []).map((l) => (l.thread_id ? `thread:${l.thread_id}:${l.label}` : `${l.doc_id}:${l.node_id}:${l.label}`)), ...threads.map((t) => `thread:${t.thread}:question`)]
      omni(['story', 'edit', key, ...links.flatMap((l) => ['--link', l])])
      write('block', key)
    }
    for (const gap of gaps.filter((g) => g.blocking === false)) {
      write('decision', key, ['--question', gap.question, '--choice', gap.recommendation, '--by', 'agent'])
    }
    seen[key] = readStory(key).version
    fs.writeFileSync(clarityPath, JSON.stringify(seen, null, 2) + '\n')
    clarity.push({ key, blocked: threads.length > 0, threads: threads.map((t) => t.thread), decisions: gaps.filter((g) => g.blocking === false).length })
  }

  const pickable = omni(['story', 'list', '--pickable']).split('\n').filter(Boolean).length
  return { run: runId, feature, integration, thread: threadId, implementers, reviewers, storyLimit: storyLimit === Infinity ? null : storyLimit, reconciled, clarity, pickable }
} finally {
  fs.rmSync(lockPath, { force: true })
}
