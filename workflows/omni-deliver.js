export const meta = {
  name: 'omni-deliver',
  description: 'Delivers a feature\'s stories from the OmniState board: locks the main worktree, reconciles claims left by dead runs, and clarity-checks the backlog before any worktree exists',
  whenToUse: 'Run from the main worktree of a project with a clean tree, with args {feature: "<name>"} (optional: implementers (3), reviewers (3), storyLimit, models). One run per checkout, whatever the feature: the lock file .omni/run.lock holds the run, and a live pid refuses a second one. Needs the omni CLI on PATH with a project in .omni.toml. Stories come from `omni story`; questions for the human become threads on the feature named after the story. Never pushes.',
  phases: [
    { title: 'Start', detail: 'clean tree, lock, the Run thread' },
    { title: 'Reconcile', detail: 'release claims of other runs, then claim, reset or keep each story by its status' },
    { title: 'Clarity', detail: 'one check per backlog story whose version is new, in parallel: blocking gaps become threads, minor ones agent decisions' },
    { title: 'Unblock', detail: 'a blocked story whose linked threads all have a conclusion gets each conclusion as a human decision and is unblocked' },
    { title: 'Implement', detail: 'up to `implementers` agents at once, each on a pickable story in its own worktree; the story reaches review only after the fast command passes there' },
    { title: 'Merge', detail: 'one worker takes the testing stories in dependency order: stack_up once per batch, then rebase in the worktree and fast-forward the integration branch; a rebase conflict is fixed by an implementer and halts merging until that story is merged' },
    { title: 'Check', detail: 'after each merge a verifier runs the story\'s criteria on the main worktree and the fast command runs; a failure becomes a priority-0 fix story that is implemented, reviewed, merged and checked before anything else merges; the stack command runs once after the batch' },
    { title: 'Review', detail: 'up to `reviewers` reviewers at once read each review story\'s branch against its criteria; a fail is fixed in place by an implementer, three fails block the story, a pass moves it to testing' },
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
const os = await import('node:os')

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

  // A question for the human is a thread on the feature named after the story,
  // linked from the story, and the story is blocked until it concludes.
  const openThread = (title, body) => {
    const id = omni(['thread', 'create', title]).trim()
    omni(['post', id, '--summary', title.slice(0, 200)], { input: body, stdio: ['pipe', 'pipe', 'pipe'] })
    return id
  }
  const linkAndBlock = (key, threadIds) => {
    const fresh = readStory(key)
    const links = [...(fresh.links || []).map((l) => (l.thread_id ? `thread:${l.thread_id}:${l.label}` : `${l.doc_id}:${l.node_id}:${l.label}`)), ...threadIds.map((id) => `thread:${id}:question`)]
    omni(['story', 'edit', key, ...links.flatMap((l) => ['--link', l])])
    write('block', key)
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
      const summary = `Options and recommendation for ${key}`.slice(0, 200)
      const body = `${gap.question}\n\nCriterion: ${gap.criterion}\n\nOptions:\n${gap.options.map((o, i) => `${i + 1}. ${o}`).join('\n')}\n\nRecommendation: ${gap.recommendation}\n\nWhy: ${gap.reason}\n`
      const id = omni(['thread', 'create', `${key}: ${gap.question}`]).trim()
      omni(['post', id, '--summary', summary], { input: body, stdio: ['pipe', 'pipe', 'pipe'] })
      threads.push({ thread: id, question: gap.question })
    }
    if (threads.length) linkAndBlock(key, threads.map((t) => t.thread))
    for (const gap of gaps.filter((g) => g.blocking === false)) {
      write('decision', key, ['--question', gap.question, '--choice', gap.recommendation, '--by', 'agent'])
    }
    seen[key] = readStory(key).version
    fs.writeFileSync(clarityPath, JSON.stringify(seen, null, 2) + '\n')
    clarity.push({ key, blocked: threads.length > 0, threads: threads.map((t) => t.thread), decisions: gaps.filter((g) => g.blocking === false).length })
  }

  // ---- unblock: a thread's conclusion is the human's answer ----------------
  // Runs after clarity at start and at the top of every loop pass.
  const reviewed = new Set()
  const resumed = []
  const unblockPass = () => {
    phase('Unblock')
    const done = []
    for (const story of listStories()) {
      if (!story.blocked || story.status === 'done' || story.status === 'cancelled') continue
      const ids = (story.links || []).map((l) => l.thread_id).filter(Boolean)
      if (ids.length === 0) continue
      const threads = ids.map((id) => {
        const parsed = JSON.parse(omni(['thread', 'read', id, '--json']))
        return parsed.thread || parsed
      })
      if (threads.some((t) => !t.conclusion)) continue
      const recorded = (story.decisions || []).filter((d) => d.by === 'human')
      for (const t of threads) {
        if (recorded.some((d) => d.question === t.question && d.choice === t.conclusion)) continue
        write('decision', story.key, ['--question', t.question, '--choice', t.conclusion, '--by', 'human'])
      }
      write('unblock', story.key)
      if (story.status === 'implementing' || story.status === 'review') {
        write('claim', story.key, ['--stage', story.status])
        if (story.status === 'review') reviewed.delete(story.key)
        resumed.push({ key: story.key, stage: story.status })
      }
      done.push(story.key)
    }
    return done
  }
  const unblocked = unblockPass()

  // ---- phase 4: implement in worktrees ------------------------------------
  phase('Implement')
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
  let fast = null
  let stackUp = ''
  let stackCmd = ''
  const docCache = {}
  const docText = (id) => {
    if (!(id in docCache)) docCache[id] = tryRun('omni', ['doc', 'read', id], { env: { ...process.env, OMNI_RUN: runId, OMNI_ROLE: 'deliver' } }) || '(the doc could not be read)'
    return docCache[id]
  }
  const runCmd = (cmd, cwd) => new Promise((resolve) => {
    if (!cmd.trim()) return resolve({ ok: true, output: '' })
    let output = ''
    const child = cp.spawn(cmd, { cwd, shell: true })
    child.stdout.on('data', (d) => { output += d })
    child.stderr.on('data', (d) => { output += d })
    child.on('error', (e) => resolve({ ok: false, output: output + String(e.message) }))
    child.on('close', (code) => resolve({ ok: code === 0, output }))
  })
  const runFast = (cwd) => runCmd(fast, cwd)
  const implPrompt = (story, wt, retry, task) => {
    const open = (story.findings || []).filter((f) => !f.resolved)
    const docs = (story.links || []).filter((l) => l.doc_id)
    return `You implement ONE story of the feature "${feature}" in its own git worktree.

## Your worktree
${wt} (branch story/${story.key}, from ${integration}). Work only there: cd into it first and run every command there.

## The story
${story.key}: ${story.title}
${story.text || ''}

### Acceptance criteria
${(story.criteria || []).map((c) => `- ${c.text || c}`).join('\n')}

### Files planned
${(story.files_planned || []).join('\n') || '(none named)'}

### Decisions already taken (settled: build against them as if the spec said so)
${(story.decisions || []).map((d) => `- ${d.question} -> ${d.choice} (${d.by})`).join('\n') || '(none)'}

### Open findings (fix all of them)
${open.map((f) => `- [${f.stage}] ${f.text}${(f.files || []).length ? ` (${f.files.join(', ')})` : ''}`).join('\n') || '(none)'}

## Linked doc nodes
${docs.map((l) => `### Doc ${l.doc_id}, node ${l.node_id}${l.label ? ` (${l.label})` : ''}\n${docText(l.doc_id)}`).join('\n\n') || '(none)'}
${retry ? `\n## The fast command failed on your last work (fix it, then commit with a fix: commit)\n\`${fast}\` run in the worktree printed:\n\`\`\`\n${retry}\n\`\`\`\n` : ''}${task ? `\n## Your task now\n${task}\n` : ''}${CONVENTIONS}
## Rules
- Implement only this story. No unrelated cleanup, no groundwork for later stories.
- Commit a feat: commit, then fix: commits for later corrections. Commit production and test files that belong to the story.
- Run the fast command before you finish: \`${fast || '(none configured)'}\`. The workflow runs it again after you return.
- If the story needs a product decision the spec does not make and the story cannot avoid, STOP: status needs_decision with question, options and recommendation. Never guess a product rule. Ordinary technical choices are yours.
- If you genuinely cannot continue, status blocked with the reason in summary.
- Return decisions for every non-obvious technical choice.
Return raw structured data only.`
  }

  const outcomes = []
  const skipped = new Set()
  let picked = 0
  const pickNext = () => {
    const keys = omni(['story', 'list', '--pickable']).split('\n').map((l) => l.trim().split(/\s+/)[0]).filter(Boolean)
    return keys.find((k) => !skipped.has(k)) || null
  }
  const ensureFast = () => {
    if (fast === null) {
      const checks = JSON.parse(omni(['checks', '--json']))
      stackUp = String(checks.stack_up || '')
      stackCmd = String(checks.stack || '')
      fast = String(checks.fast || '')
    }
  }
  // Runs the implementer until the fast command passes in the worktree, then
  // rewrites files_changed. Returns null when it did, else the outcome of the
  // thread that blocked the story.
  const implementLoop = async (key, wt, story, task) => {
    const recorded = new Set()
    const record = (r) => {
      for (const d of r.decisions || []) {
        const id = `${d.question}\n${d.choice}`
        if (recorded.has(id)) continue
        recorded.add(id)
        write('decision', key, ['--question', d.question, '--choice', d.choice, '--by', 'agent'])
      }
    }
    let retry = null
    for (let attempt = 1; attempt <= 3; attempt++) {
      const r = (await runAgent(implPrompt(story, wt, retry, task), { schema: IMPL_SCHEMA, label: `impl:${key}${attempt > 1 ? ` fix${attempt - 1}` : ''}`, phase: 'Implement', model: modelFor('impl'), role: 'impl' })) || { status: 'blocked', summary: 'The implementer returned no result (it may have died).' }
      record(r)
      if (r.status === 'needs_decision') {
        const options = (r.options || []).map((o, i) => `${i + 1}. ${o}`).join('\n')
        const id = openThread(`${key}: ${r.question}`, `${r.question}\n\nOptions:\n${options}\n\nRecommendation: ${r.recommendation || ''}\n`)
        linkAndBlock(key, [id])
        return { key, outcome: 'needs_decision', thread: id }
      }
      if (r.status !== 'ready') {
        const id = openThread(`${key}: the implementer is blocked`, `${r.summary}\n`)
        linkAndBlock(key, [id])
        return { key, outcome: 'blocked', thread: id }
      }
      const result = await runFast(wt)
      if (result.ok) {
        const paths = git(['-C', wt, 'diff', '--name-only', `${integration}...HEAD`]).split('\n').filter(Boolean)
        if (paths.length) write('files', key, paths)
        return null
      }
      retry = result.output
    }
    const id = openThread(`${key}: the fast command still fails after three attempts`, `Last output of \`${fast}\`:\n\n\`\`\`\n${retry}\n\`\`\`\n`)
    linkAndBlock(key, [id])
    return { key, outcome: 'fast_failed', thread: id }
  }
  const implement = async (key) => {
    const bad = await implementLoop(key, worktreeOf(key), readStory(key))
    if (bad) return bad
    write('move', key, ['--to', 'review'])
    return { key, outcome: 'review' }
  }
  const start = (key) => {
    ensureFast()
    dropWorktree(key)
    write('claim', key, ['--stage', 'implementing'])
    fs.mkdirSync(storiesDir, { recursive: true })
    git(['worktree', 'add', worktreeOf(key), '-b', `story/${key}`, integration])
    return implement(key).catch((e) => ({ key, outcome: 'failed', error: String(e && e.message || e) }))
  }

  // ---- review definitions (used by the merge worker for fix stories) ------
  const REVIEW_SCHEMA = {
    type: 'object',
    required: ['verdict', 'findings'],
    properties: {
      verdict: { type: 'string', enum: ['pass', 'fail', 'needs_decision'] },
      findings: {
        type: 'array',
        description: 'Defects to fix, one per problem. Empty on pass',
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
  }
  const reviewPrompt = (story, wt) => {
    const docs = (story.links || []).filter((l) => l.doc_id)
    return `You review ONE story of the feature "${feature}" before it merges. You read; you run nothing and change nothing.

## The change
Worktree ${wt}, branch story/${story.key}, from ${integration}. See the diff with \`git -C ${wt} diff ${integration}...HEAD\` and read the changed files there.

## The story
${story.key}: ${story.title}
${story.text || ''}

### Acceptance criteria
${(story.criteria || []).map((c) => `- ${c.text || c}`).join('\n')}

### Files planned
${(story.files_planned || []).join('\n') || '(none named)'}

### Decisions already taken (settled: judge against them as if the spec said so)
${(story.decisions || []).map((d) => `- ${d.question} -> ${d.choice} (${d.by})`).join('\n') || '(none)'}

## Linked doc nodes
${docs.map((l) => `### Doc ${l.doc_id}, node ${l.node_id}${l.label ? ` (${l.label})` : ''}\n${docText(l.doc_id)}`).join('\n\n') || '(none)'}

## Rules
- Read the diff and the files against every criterion, the interfaces and the linked doc nodes. Do not run tests, builds or the app.
- verdict fail: one finding per defect, each with the files it is in and the criterion it breaks.
- verdict needs_decision: only when the spec does not decide the behaviour and the story cannot avoid it; give question, options and recommendation.
- verdict pass: every criterion is met by the code you read. Findings empty.
Return raw structured data only.`
  }
  const openReviewFindings = (key) => (readStory(key).findings || []).map((f, i) => ({ ...f, i })).filter((f) => f.stage === 'review' && !f.resolved)
  const resolveReviewFindings = (key) => {
    for (const f of openReviewFindings(key)) write('resolve', key, ['--finding', String(f.i)])
  }
  const review = async (key) => {
    const wt = worktreeOf(key)
    ensureFast()
    write('claim', key, ['--stage', 'review'])
    for (let round = 1; round <= 3; round++) {
      let verdict = null
      for (let attempt = 0; attempt < 2 && !verdict; attempt++) {
        verdict = await runAgent(reviewPrompt(readStory(key), wt), { schema: REVIEW_SCHEMA, label: `review:${key}${round > 1 ? ` round${round}` : ''}`, phase: 'Review', model: modelFor('review'), role: 'review' })
      }
      if (!verdict) {
        const id = openThread(`${key}: the reviewer returned nothing`, 'The reviewer returned no result twice.\n')
        linkAndBlock(key, [id])
        return { key, outcome: 'review_failed', thread: id }
      }
      if (verdict.verdict === 'needs_decision') {
        const options = (verdict.options || []).map((o, i) => `${i + 1}. ${o}`).join('\n')
        const id = openThread(`${key}: ${verdict.question}`, `${verdict.question}\n\nOptions:\n${options}\n\nRecommendation: ${verdict.recommendation || ''}\n`)
        linkAndBlock(key, [id])
        return { key, outcome: 'needs_decision', thread: id }
      }
      if (verdict.verdict === 'pass') {
        resolveReviewFindings(key)
        write('move', key, ['--to', 'testing'])
        return { key, outcome: 'testing' }
      }
      const findings = verdict.findings || []
      // The open findings are this round's; the last round's were fixed or are superseded.
      resolveReviewFindings(key)
      for (const f of findings) write('finding', key, ['--stage', 'review', '--text', f.text, ...(f.files || []).flatMap((p) => ['--file', p])])
      if (round === 3) {
        const list = findings.map((f) => `- ${f.text}${(f.files || []).length ? ` (${f.files.join(', ')})` : ''}`).join('\n')
        const id = openThread(`${key}: the review still fails after three attempts`, `The review failed three times. Findings of the last round:\n\n${list}\n`)
        linkAndBlock(key, [id])
        return { key, outcome: 'review_failed', thread: id }
      }
      const bad = await implementLoop(key, wt, readStory(key))
      if (bad) return bad
    }
  }

  // ---- merge: one worker on the main worktree ------------------------------
  const merged = new Set()
  const mergeOutcomes = []
  let halted = null
  const REBASE_FINDING = 'rebase conflict in '
  const inOrder = (stories) => {
    const ref = new Map()
    for (const s of stories) { ref.set(s.key, s); if (s.id) ref.set(s.id, s) }
    const number = (s) => (Number.isInteger(s.number) ? s.number : Number(String(s.key).replace(/\D/g, '')))
    const rest = [...stories]
    const out = []
    while (rest.length) {
      const ready = rest.filter((s) => !(s.blocked_by || []).some((b) => ref.has(b) && rest.includes(ref.get(b)) && ref.get(b) !== s))
      const next = (ready.length ? ready : rest).sort((a, b) => (a.priority ?? 3) - (b.priority ?? 3) || number(a) - number(b))[0]
      out.push(next)
      rest.splice(rest.indexOf(next), 1)
    }
    return out
  }
  // Rebases the story in its worktree, then fast-forwards the integration
  // branch. Returns null when it merged, else the files that conflict.
  const tryMerge = (key) => {
    const wt = worktreeOf(key)
    try { git(['-C', wt, 'rebase', integration]) } catch (e) {
      const files = (tryRun('git', ['-C', wt, 'diff', '--name-only', '--diff-filter=U']) || '').split('\n').filter(Boolean)
      tryRun('git', ['-C', wt, 'rebase', '--abort'])
      return files.length ? files : ['(unknown)']
    }
    git(['merge', '--ff-only', `story/${key}`])
    merged.add(key)
    const fresh = readStory(key)
    ;(fresh.findings || []).forEach((f, i) => {
      if (f.stage === 'testing' && !f.resolved && String(f.text).startsWith(REBASE_FINDING)) write('resolve', key, ['--finding', String(i)])
    })
    return null
  }
  // ---- check: exercise a merged story, and the fix story when it fails ------
  const FAST_CRITERION = 'The fast command passes on the main worktree'
  const STACK_CRITERION = 'The stack command passes on the main worktree'
  const LINT_CRITERION = 'Typecheck/lint passes (make lint)'
  const CHECK_SCHEMA = {
    type: 'object',
    required: ['criteria'],
    properties: {
      criteria: {
        type: 'array',
        description: 'One entry per acceptance criterion in the story, in order',
        items: {
          type: 'object',
          required: ['index', 'criterion', 'ok', 'evidence'],
          properties: {
            index: { type: 'integer', description: 'The 0-based position of the criterion in the story' },
            criterion: { type: 'string', description: 'The criterion text, verbatim' },
            ok: { type: 'boolean' },
            evidence: { type: 'string', description: 'The command or steps, the input, and what you actually observed. For a failure: what is wrong and where' },
          },
        },
      },
    },
  }
  const checkPrompt = (story) => {
    const docs = (story.links || []).filter((l) => l.doc_id)
    return `You check ONE merged story of the feature "${feature}" by running it. Its branch is already merged into ${integration} on the main worktree ${root}, and the stack is up there (the stack_up command ran for this batch). You change nothing: no edits, no commits.

## The story
${story.key}: ${story.title}
${story.text || ''}

### Acceptance criteria (index: text)
${(story.criteria || []).map((c, i) => `${i}: ${c.text || c}${c.human_check ? ' [human check]' : ''}`).join('\n')}

### Decisions already taken (settled: judge against them as if the spec said so)
${(story.decisions || []).map((d) => `- ${d.question} -> ${d.choice} (${d.by})`).join('\n') || '(none)'}

## Linked doc nodes
${docs.map((l) => `### Doc ${l.doc_id}, node ${l.node_id}${l.label ? ` (${l.label})` : ''}\n${docText(l.doc_id)}`).join('\n\n') || '(none)'}

## Rules
- Exercise EVERY criterion by running the behaviour in ${root}: call the endpoint, run the command, open the page with the dev-browser skill for browser criteria. Reading code is not a check.
- A criterion about a command (fast, stack, lint) is checked by running that command: fast is \`${fast || '(none configured)'}\`, stack is \`${stackCmd || '(none configured)'}\`, lint is \`make lint\`.
- A [human check] criterion: exercise what you can; ok is true unless what you ran contradicts it. A human confirms it later.
- ok false only for a criterion the behaviour does not meet. Say what is wrong and where.
Return raw structured data only.`
  }
  // Returns {failures} (empty when everything passes) or null when the verifier returned nothing.
  const exercise = async (key) => {
    const story = readStory(key)
    let verdict = null
    for (let attempt = 0; attempt < 2 && !verdict; attempt++) {
      verdict = await runAgent(checkPrompt(story), { schema: CHECK_SCHEMA, label: `verify:${key}${attempt ? ' retry' : ''}`, phase: 'Check', model: modelFor('verify'), role: 'verify' })
    }
    if (!verdict) return null
    const failures = []
    ;(story.criteria || []).forEach((c, i) => {
      const r = (verdict.criteria || []).find((x) => x.index === i)
      if (!r || !r.ok) failures.push({ criterion: c.text || c, text: `${c.text || c}: ${r ? r.evidence : 'not exercised'}` })
    })
    const result = await runFast(root)
    if (!result.ok) failures.push({ criterion: FAST_CRITERION, text: `The fast command failed on the main worktree. Output:\n\`\`\`\n${result.output}\n\`\`\`` })
    return { story, failures }
  }
  const checkOutcomes = []
  // Ticks every criterion (human checks pending), resolves the testing
  // findings and removes the worktree and branch.
  const passCheck = (key, story) => {
    ;(story.criteria || []).forEach((c, i) => write('tick', key, ['--criterion', String(i), ...(c.human_check ? ['--pending'] : [])]))
    ;(readStory(key).findings || []).forEach((f, i) => {
      if (f.stage === 'testing' && !f.resolved) write('resolve', key, ['--finding', String(i)])
    })
    dropWorktree(key)
  }
  // Records the check's result on the story. Returns 'passed', 'failed' (with
  // the failures as testing findings) or 'unchecked' (the verifier returned
  // nothing, the story is blocked with a thread).
  const checkStory = async (key) => {
    phase('Check')
    const r = await exercise(key)
    if (!r) {
      const id = openThread(`${key}: the verifier returned nothing`, 'The verifier returned no result twice.\n')
      linkAndBlock(key, [id])
      checkOutcomes.push({ key, outcome: 'unchecked', thread: id })
      return { state: 'unchecked' }
    }
    if (r.failures.length === 0) {
      passCheck(key, r.story)
      checkOutcomes.push({ key, outcome: 'passed' })
      return { state: 'passed' }
    }
    for (const f of r.failures) write('finding', key, ['--stage', 'testing', '--text', f.text])
    checkOutcomes.push({ key, outcome: 'failed', failures: r.failures.length })
    return { state: 'failed', story: r.story, failures: r.failures }
  }
  // A line the story parser would take for a label or a heading is quoted.
  const flat = (text) => String(text).split('\n').filter((l) => l.trim()).map((l, i) => (i > 0 && /^\s*(#|- \[|\*\*)/.test(l) ? `> ${l}` : l)).join('\n')
  const docLinks = (story) => (story.links || []).filter((l) => l.doc_id).map((l) => `${l.doc_id}:${l.node_id}:${l.label || ''}`)
  // Creates a priority-0 fix story from a generated file. Returns its key.
  const addFix = ({ heading, text, files, criteria, links }) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'omni-deliver-'))
    try {
      const file = path.join(dir, 'fix.md')
      fs.writeFileSync(file, `### ${heading}\n**Description:** ${flat(text)}\n**Topic:** bugfix · fix\n**Blocked by:** none\n**Files:** change: ${files.map((f) => `\`${f}\``).join(', ') || '(none)'}\n\n**Acceptance Criteria:**\n${criteria.map((c) => `- [ ] ${c}`).join('\n')}\n`)
      const key = omni(['story', 'add', file, '--priority', '0', ...[...new Set(links)].flatMap((l) => ['--link', l])]).trim()
      // The merge worker drives the fix story itself, so the implement loop leaves it alone.
      skipped.add(key)
      return key
    } finally { fs.rmSync(dir, { recursive: true, force: true }) }
  }
  const waitingFinding = (key, fix) => (readStory(key).findings || []).map((f, i) => ({ ...f, i })).find((f) => f.stage === 'testing' && !f.resolved && f.text === `waiting for ${fix}`)
  // Implements, reviews, merges and checks a fix story. Returns {state: 'passed'}
  // or {state: 'halt'} when it could not be merged, {state: 'failed', failures}
  // or {state: 'unchecked'} from its check.
  const driveFix = async (fix) => {
    let o
    try { o = await start(fix) } catch (e) { o = { key: fix, outcome: 'failed', error: String(e && e.message || e) } }
    if (o.outcome !== 'review') { mergeOutcomes.push({ ...o, stage: 'fix' }); return { state: 'halt' } }
    const reviewedFix = await review(fix).catch((e) => ({ key: fix, outcome: 'failed', error: String(e && e.message || e) }))
    reviewed.add(fix)
    if (reviewedFix.outcome !== 'testing') { mergeOutcomes.push({ ...reviewedFix, stage: 'fix' }); return { state: 'halt' } }
    if (!(await mergeStory(fix))) return { state: 'halt' }
    return checkStory(fix)
  }
  // Checks a story just merged. Returns false when merging has to stop.
  const checkMerged = async (key, mergedKeys) => {
    const first = await checkStory(key)
    if (first.state !== 'failed') return true
    const fix = addFix({
      heading: `Fix ${key}: ${String(first.failures[0].criterion).slice(0, 80)}`,
      text: first.failures.map((f) => f.text).join('\n'),
      files: first.story.files_changed || [],
      criteria: [...new Set([...first.failures.map((f) => f.criterion), LINT_CRITERION])],
      links: docLinks(first.story),
    })
    write('finding', key, ['--stage', 'testing', '--text', `waiting for ${fix}`])
    const fixed = await driveFix(fix)
    if (fixed.state === 'halt') { halted = fix; return false }
    mergedKeys.push(fix)
    if (fixed.state === 'unchecked') return false
    if (fixed.state === 'failed') {
      const list = (title, failures) => `${title}\n${failures.map((f) => `- ${f.text}`).join('\n')}`
      const id = openThread(`${key}: the fix ${fix} fails its own check`, `${list(`Findings of ${key}:`, first.failures)}\n\n${list(`Findings of ${fix}:`, fixed.failures)}\n`)
      linkAndBlock(key, [id])
      linkAndBlock(fix, [id])
      checkOutcomes.push({ key, outcome: 'fix_failed', fix, thread: id })
      return true
    }
    const again = await checkStory(key)
    if (again.state === 'failed') {
      const w = waitingFinding(key, fix)
      if (w) write('resolve', key, ['--finding', String(w.i)])
      const id = openThread(`${key}: the check still fails after ${fix}`, `${again.failures.map((f) => `- ${f.text}`).join('\n')}\n`)
      linkAndBlock(key, [id])
      checkOutcomes.push({ key, outcome: 'still_failing', fix, thread: id })
    }
    return true
  }
  // After the last story of a batch: the stack command once.
  const checkBatch = async (mergedKeys) => {
    const result = await runCmd(stackCmd, root)
    if (result.ok) return true
    const stories = mergedKeys.map((k) => readStory(k))
    const fix = addFix({
      heading: `Fix batch ${mergedKeys.join(', ')}`,
      text: `The stack command \`${stackCmd}\` failed on the main worktree after merging ${mergedKeys.join(', ')}. Output:\n\`\`\`\n${result.output}\n\`\`\``,
      files: [...new Set(stories.flatMap((s) => s.files_changed || []))],
      criteria: [STACK_CRITERION, LINT_CRITERION],
      links: stories.flatMap(docLinks),
    })
    const fixed = await driveFix(fix)
    if (fixed.state === 'halt') { halted = fix; return false }
    if (fixed.state === 'failed') {
      const id = openThread(`${fix}: the batch fix fails its own check`, `Output of \`${stackCmd}\`:\n\n\`\`\`\n${result.output}\n\`\`\`\n\nFindings:\n${fixed.failures.map((f) => `- ${f.text}`).join('\n')}\n`)
      linkAndBlock(fix, [id])
      checkOutcomes.push({ key: fix, outcome: 'batch_fix_failed', thread: id })
    }
    return true
  }
  // Rebases the story and fast-forwards it; a conflict is fixed by an implementer.
  // Returns false when merging has to stop.
  const mergeStory = async (key) => {
    const conflict = tryMerge(key)
    if (!conflict) { mergeOutcomes.push({ key, outcome: 'merged' }); return true }
    halted = key
    write('finding', key, ['--stage', 'testing', '--text', `${REBASE_FINDING}${conflict.join(', ')}`, ...conflict.flatMap((p) => ['--file', p])])
    write('move', key, ['--to', 'implementing'])
    const wt = worktreeOf(key)
    const task = `Story branch story/${key} no longer rebases onto ${integration} (${conflict.join(', ')}). In ${wt}, run \`git rebase ${integration}\`, resolve the conflicts, finish the rebase, and run the fast command again. Do not push and do not touch the main worktree.`
    const bad = await implementLoop(key, wt, readStory(key), task)
    if (bad) { mergeOutcomes.push({ ...bad, stage: 'rebase' }); return false }
    write('move', key, ['--to', 'testing'])
    const again = tryMerge(key)
    if (again) {
      const id = openThread(`${key}: the rebase still conflicts`, `Rebasing story/${key} onto ${integration} still conflicts in ${again.join(', ')} after the fix.\n`)
      linkAndBlock(key, [id])
      mergeOutcomes.push({ key, outcome: 'rebase_failed', thread: id })
      return false
    }
    halted = null
    mergeOutcomes.push({ key, outcome: 'merged', rebased: true })
    return true
  }
  const mergePass = async () => {
    if (halted) return
    const batch = inOrder(listStories().filter((s) => s.status === 'testing' && !s.blocked && s.holder && s.holder.run === runId && !merged.has(s.key) && fs.existsSync(worktreeOf(s.key))))
    if (batch.length === 0) return
    phase('Merge')
    ensureFast()
    const up = await runCmd(stackUp, root)
    if (!up.ok) {
      openThread('The stack did not start', `\`${stackUp}\` exited non-zero, so nothing from this batch was merged (${batch.map((s) => s.key).join(', ')}). Output:\n\n\`\`\`\n${up.output}\n\`\`\`\n`)
      mergeOutcomes.push({ key: null, outcome: 'stack_up_failed' })
      return
    }
    const mergedKeys = []
    for (const { key } of batch) {
      if (!(await mergeStory(key))) return
      mergedKeys.push(key)
      if (!(await checkMerged(key, mergedKeys))) return
    }
    await checkBatch(mergedKeys)
  }
  let mergeChain = Promise.resolve()
  const kickMerge = () => {
    mergeChain = mergeChain.then(mergePass).catch((e) => {
      mergeOutcomes.push({ key: halted, outcome: 'failed', error: String(e && e.message || e) })
      log(`merge: ${e && e.message || e}`)
    })
    return mergeChain
  }
  kickMerge()

  const active = new Map()
  for (;;) {
    while (active.size < implementers && picked < storyLimit) {
      const key = pickNext()
      if (!key) break
      picked++
      let p
      try { p = start(key) } catch (e) {
        skipped.add(key)
        outcomes.push({ key, outcome: 'failed', error: String(e && e.message || e) })
        continue
      }
      active.set(key, p.then((o) => ({ key, o })))
    }
    if (active.size === 0) break
    const { key, o } = await Promise.race(active.values())
    active.delete(key)
    outcomes.push(o)
    if (o.error) log(`${key}: ${o.error}`)
  }

  // ---- phase 5: review in the worktree -------------------------------------
  phase('Review')
  const reviewOutcomes = []
  const queue = listStories().filter((s) => s.status === 'review' && !s.blocked && !reviewed.has(s.key) && !skipped.has(s.key)).map((s) => s.key)
  await Promise.all(Array.from({ length: Math.min(reviewers, queue.length) }, async () => {
    while (queue.length) {
      const key = queue.shift()
      const o = await review(key).catch((e) => ({ key, outcome: 'failed', error: String(e && e.message || e) }))
      reviewed.add(key)
      reviewOutcomes.push(o)
      kickMerge()
      if (o.error) log(`${key}: ${o.error}`)
    }
  }))

  kickMerge()
  await mergeChain

  const pickable = omni(['story', 'list', '--pickable']).split('\n').filter(Boolean).length
  return { run: runId, feature, integration, thread: threadId, implementers, reviewers, storyLimit: storyLimit === Infinity ? null : storyLimit, reconciled, clarity, unblocked, resumed, implemented: outcomes, reviewed: reviewOutcomes, merged: mergeOutcomes, checked: checkOutcomes, pickable }
} finally {
  fs.rmSync(lockPath, { force: true })
}
