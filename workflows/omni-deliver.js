export const meta = {
  name: 'omni-deliver',
  description: 'Delivers a feature\'s stories from the OmniState board with three loops (implement, review, test) and a single test lane: locks the main worktree, reconciles claims left by dead runs, and clarity-checks the backlog before any worktree exists',
  whenToUse: 'Run from the main worktree of a project with a clean tree, with args {feature: "<name>"} (optional: implementers (3), reviewers (3), storyLimit, models, timeouts). One run per checkout, whatever the feature: the lock file .omni/run.lock holds the run, and a live pid refuses a second one. Needs the omni CLI on PATH and the folder linked to its project (omni link, or OMNI_PROJECT). Stories come from `omni story`; questions for the human become threads on the feature named after the story. A run has three loops, implement, review and test, that pull stories from the board by status, and the test step is a single lane: one story at a time on the main worktree. A git or omni call that runs past 1 minute fails with an error that says it timed out, and a shell command (fast, stack_up, stack, the test author\'s check) that runs past 45 minutes is stopped with everything it started and fails; args.timeouts sets any of these limits by name, in minutes (call, fast, stack_up, stack, check), and the Node runner reads its agent sessions\' limits from the same map. Never pushes.',
  phases: [
    { title: 'Start', detail: 'clean tree, lock, the Run thread' },
    { title: 'Reconcile', detail: 'undo a merge a dead run left half done (the lane file), then release every story held by another run, and every working story nobody holds' },
    { title: 'Clarity', detail: 'one check per backlog story whose version is new, in parallel: blocking gaps become threads, minor ones agent decisions' },
    { title: 'Unblock', detail: 'a blocked story whose linked threads all have a conclusion gets each conclusion as a human decision and is unblocked' },
    { title: 'Implement', detail: 'the implement loop polls the board every 5 seconds and when an implementer ends: while fewer than `implementers` run, it claims the first pickable story that passed clarity and starts an implementer in its worktree (an existing one is continued), writing no tests and running only the existing checks its change needs; a finished story moves to implemented, a blocked one gets a thread and stays in the backlog' },
    { title: 'Test lane', detail: 'one story at a time on the main worktree: the lane claims the first reviewed story, writes .omni/lane.json, rebases its branch onto the integration branch and fast-forwards it, runs the fast command, then the stack (a command past its time limit is stopped and fails), then a verifier on its criteria, then a test author writes its regression tests by the writing-tests skill and the lane runs them (a red run drops the test commits and the report says so), and it moves to done; a failure before the tests resets the branch to the lane base and returns the story to the backlog with the findings at priority 0, its branch kept' },
    { title: 'Review', detail: 'up to `reviewers` reviewers at once read each review story\'s branch against its criteria; a fail returns the story to the backlog at priority 0 with the findings, for a fresh implementer that continues from the story\'s branch, and a fresh reviewer; a third failed attempt opens a thread and blocks the story; a pass moves it to reviewed' },
    { title: 'Report', detail: 'claims on unfinished stories are released, a verifier runs the fast command and the journeys of the stories done, and the report is posted on the Run thread' },
  ],
}

// ---- args ------------------------------------------------------------------
const A = (args && typeof args === 'object') ? args : {}
if (!A.feature) throw new Error('Pass the feature: args {feature: "<name>"}')
const feature = A.feature
const implementers = A.implementers || 3
const reviewers = A.reviewers || 3
const storyLimit = A.storyLimit || Infinity
const POLL_MS = 5000
// ---- time limits -----------------------------------------------------------
// Minutes: `call` for each git and omni call, the others for the shell
// command of that name. args.timeouts overrides any of them; the Node runner
// takes its agent sessions' limits from the same map.
const LIMITS = { call: 1, fast: 45, stack_up: 45, stack: 45, check: 45 }
const limitMs = (name) => {
  const minutes = Number((A.timeouts || {})[name])
  return 60000 * (minutes > 0 ? minutes : LIMITS[name])
}
// A stopped command gets SIGTERM first, and SIGKILL this much later.
const KILL_GRACE_MS = 10000
// A limit as the log and findings show it: minutes, or seconds below a minute.
const span = (ms) => (ms >= 60000 ? `${+(ms / 60000).toFixed(1)} min` : `${+(ms / 1000).toFixed(1)}s`)
// ---- model routing per role ------------------------------------------------
// Same mapping as ralph-spec: Sonnet builds, Opus judges. Override per role with
// args.models, or force one model everywhere with args.model.
const MODELS = { clarity: 'opus', impl: 'sonnet', implRetry: 'opus', review: 'opus', verify: 'opus', tests: 'opus', wrap: 'sonnet', ...(A.models || {}) }
const modelFor = (role) => A.model || MODELS[role]
const AGENT_TYPES = { clarity: 'verifier', impl: 'implementer', review: 'verifier', verify: 'verifier', tests: 'implementer', wrap: 'verifier', ...(A.agentTypes || {}) }
// The role goes along: the Node runner sets the session's time limit by it.
const runAgent = (prompt, opts = {}) =>
  agent(prompt, AGENT_TYPES[opts.role] ? { ...opts, agentType: AGENT_TYPES[opts.role] } : opts)

// ---- git and omni through node:child_process -------------------------------
const cp = await import('node:child_process')
const fs = await import('node:fs')
const path = await import('node:path')
const os = await import('node:os')

// Every call has a time limit: a synchronous `omni post` once held a run for
// 76 minutes. Past it the call is killed, and the error says it timed out.
const run = (cmd, argv, opts = {}) => {
  const timeout = opts.timeout || limitMs('call')
  try {
    return cp.execFileSync(cmd, argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts, timeout })
  } catch (e) {
    if (e && e.code === 'ETIMEDOUT') throw new Error(`\`${[cmd, ...argv].join(' ')}\` timed out after ${span(timeout)}`)
    throw e
  }
}
const tryRun = (cmd, argv, opts) => { try { return run(cmd, argv, opts) } catch { return null } }
// The process groups of the shell commands running now (runCmd). A detached
// group no longer gets the terminal's Ctrl-C, so what still runs when the
// run ends, or the process exits, is stopped.
const groups = new Set()
const stopGroups = () => { for (const pid of groups) { try { process.kill(-pid, 'SIGTERM') } catch {} } }
const root = run('git', ['rev-parse', '--show-toplevel']).trim()
const git = (argv, opts) => run('git', ['-C', root, ...argv], opts)
const repoName = path.basename(root)
const storiesDir = path.join(root, '..', `${repoName}-stories`)
// Each worktree keeps its own cargo target directory. Sharing one across
// worktrees looked cheaper, but a test binary bakes CARGO_MANIFEST_DIR in at
// compile time and cargo reuses it across worktrees, so the main worktree's
// tests ended up looking for a worktree that was gone.
const lockPath = path.join(root, '.omni', 'run.lock')
const clarityPath = path.join(root, '.omni', 'clarity.json')
const LANE_FILE = path.join(root, '.omni', 'lane.json')

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
const decisionsTaken = []
const write = (verb, key, extra = []) => {
  if (verb === 'decision') {
    const at = (flag) => extra[extra.indexOf(flag) + 1]
    decisionsTaken.push({ key, question: at('--question'), choice: at('--choice'), by: at('--by') })
  }
  return omni(['story', verb, key, '--run', runId, ...extra])
}

process.on('exit', stopGroups)
try {
  log(`Run ${runId} on feature ${feature}, integration branch ${integration}`)
  const threadId = omni(['thread', 'create', `Run ${runId}`]).trim()

  // ---- phase 2: reconcile --------------------------------------------------
  phase('Reconcile')
  const released = []
  const worktreeOf = (key) => path.join(storiesDir, key)
  const dropWorktree = (key) => {
    const wt = worktreeOf(key)
    if (fs.existsSync(wt)) {
      // Removing a worktree deletes its build output too, which takes minutes.
      if (tryRun('git', ['-C', root, 'worktree', 'remove', '--force', wt], { timeout: Math.max(limitMs('call'), 10 * 60000) }) === null) fs.rmSync(wt, { recursive: true, force: true })
    }
    tryRun('git', ['-C', root, 'worktree', 'prune'])
    tryRun('git', ['-C', root, 'branch', '-D', `story/${key}`])
    tryRun('git', ['-C', root, 'config', '--unset', `branch.story/${key}.omnibase`])
  }
  // A lane file means a dead run died inside the test lane: undo what it merged,
  // unless its story is done, and put the story back to reviewed.
  const readLane = () => {
    let raw
    try { raw = fs.readFileSync(LANE_FILE, 'utf8') } catch { return null }
    try {
      const lane = JSON.parse(raw)
      if (lane && typeof lane.key === 'string' && typeof lane.base === 'string') return { key: lane.key, base: lane.base }
    } catch {}
    log('The lane file cannot be read; removing it without a reset')
    fs.rmSync(LANE_FILE, { force: true })
    return null
  }
  const applyLane = () => {
    const lane = readLane()
    if (!lane) return
    let story = null
    try { story = readStory(lane.key) } catch {}
    if (!story || story.status !== 'done') git(['reset', '--hard', lane.base])
    if (story && story.status !== 'done' && story.status !== 'cancelled' && (story.holder || ['implementing', 'review', 'testing'].includes(story.status))) {
      write('release', lane.key)
      released.push({ key: lane.key, status: readStory(lane.key).status })
    }
    fs.rmSync(LANE_FILE, { force: true })
  }
  applyLane()
  // A working story nobody holds (a claim whose run died before it could be
  // released) is released as well: no run can move or claim it otherwise.
  const WORKING = ['implementing', 'review', 'testing']
  for (const story of listStories()) {
    if (story.holder ? story.holder.run === runId : !WORKING.includes(story.status)) continue
    write('release', story.key)
    released.push({ key: story.key, status: readStory(story.key).status })
  }

  // A post's body goes through a file: fed through stdin, the synchronous
  // spawn once left `omni post` waiting on its stdin for over an hour, and the
  // run with it.
  const post = (id, summary, body) => {
    const file = path.join(os.tmpdir(), `omni-post-${id}.md`)
    fs.writeFileSync(file, body)
    try {
      omni(['post', id, '--summary', summary, '--body-file', file])
    } finally {
      try { fs.unlinkSync(file) } catch {}
    }
  }
  // A question for the human is a thread on the feature named after the story,
  // linked from the story, and the story is blocked until it concludes.
  const openThread = (title, body) => {
    const id = omni(['thread', 'create', title]).trim()
    post(id, title.slice(0, 200), body)
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
  // A story whose check returned nothing has not passed clarity: the implement loop leaves it.
  const clarityFailed = new Set()
  for (const result of checked) {
    if (!result || result.gaps === null) {
      log(`Clarity check returned nothing for ${result ? result.story.key : '?'}; it is checked again at the next start`)
      if (result) clarityFailed.add(result.story.key)
      continue
    }
    const { story, gaps } = result
    const key = story.key
    const threads = []
    for (const gap of gaps.filter((g) => g.blocking !== false)) {
      const summary = `Options and recommendation for ${key}`.slice(0, 200)
      const body = `${gap.question}\n\nCriterion: ${gap.criterion}\n\nOptions:\n${gap.options.map((o, i) => `${i + 1}. ${o}`).join('\n')}\n\nRecommendation: ${gap.recommendation}\n\nWhy: ${gap.reason}\n`
      const id = omni(['thread', 'create', `${key}: ${gap.question}`]).trim()
      post(id, summary, body)
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
      if (story.status === 'implementing' || story.status === 'review' || story.status === 'testing') {
        write('claim', story.key, ['--stage', story.status])
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
  // Runs a shell command within the time limit of its name. The command gets a
  // process group of its own, so that the limit stops everything it started:
  // SIGTERM to the group, then SIGKILL. A stopped command fails, and its
  // output ends with the limit it ran into.
  const runCmd = (cmd, cwd, name) => new Promise((resolve) => {
    if (!cmd.trim()) return resolve({ ok: true, output: '' })
    const limit = limitMs(name)
    let output = ''
    let timedOut = false
    let killer = null
    // No stdin: a command that reads it ends at once, where an open pipe that
    // nothing writes to held it until its limit.
    const child = cp.spawn(cmd, { cwd, shell: true, detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
    if (child.pid) groups.add(child.pid)
    const signal = (sig) => { try { process.kill(-child.pid, sig) } catch {} }
    const timer = setTimeout(() => {
      timedOut = true
      signal('SIGTERM')
      killer = setTimeout(() => {
        signal('SIGKILL')
        // A process that left the group may still hold the pipes open.
        child.stdout.destroy()
        child.stderr.destroy()
      }, KILL_GRACE_MS)
    }, limit)
    const finish = (result) => {
      clearTimeout(timer)
      clearTimeout(killer)
      groups.delete(child.pid)
      resolve(result)
    }
    child.stdout.on('data', (d) => { output += d })
    child.stderr.on('data', (d) => { output += d })
    child.on('error', (e) => finish({ ok: false, output: output + String(e.message) }))
    child.on('close', (code) => finish(timedOut
      ? { ok: false, timedOut: true, output: `${output}\nTimed out: the ${name} command ran past its limit of ${span(limit)} (args.timeouts.${name}) and was stopped.\n` }
      : { ok: code === 0, output }))
  })
  const runFast = (cwd) => runCmd(fast, cwd, 'fast')
  const implPrompt = (story, wt, task) => {
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
${task ? `\n## Your task now\n${task}\n` : ''}${CONVENTIONS}
## Rules
- Implement only this story. No unrelated cleanup, no groundwork for later stories.
- Commit a feat: commit, then fix: commits for later corrections. Commit the story's production files.
- Write NO tests: no new test, test file, test helper, fixture or snapshot. The lane's test author writes the story's tests from the story text, by the writing-tests skill, after a verifier has confirmed the story works. The one exception: an existing test that asserts the behaviour this story changes may be updated to the story's decision; name it in decisions.
- Before you finish, run the narrowest existing check that proves your change: one crate's existing tests, one package's typecheck. Do not run the full check command here; it runs on the main worktree after the merge.
- The stack tests run ONLY as a read-only check through the configured stack command's compose project, \`${stackCmd || '(none configured)'}\`, from the main worktree's folder ${root}: never \`docker compose up\` or \`docker compose run\` from your worktree. The compose project is shared, and a run from your worktree recreates the stack the test lane depends on. When you cannot run them that way, say so in decisions instead of running them.
- If the story needs a product decision the spec does not make and the story cannot avoid, STOP: status needs_decision with question, options and recommendation. Never guess a product rule. Ordinary technical choices are yours.
- If you genuinely cannot continue, status blocked with the reason in summary.
- Return decisions for every non-obvious technical choice.
Return raw structured data only.`
  }

  const outcomes = []
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
  const runImplementer = async (key, wt, story, task) => {
    const recorded = new Set()
    const record = (r) => {
      for (const d of r.decisions || []) {
        const id = `${d.question}\n${d.choice}`
        if (recorded.has(id)) continue
        recorded.add(id)
        write('decision', key, ['--question', d.question, '--choice', d.choice, '--by', 'agent'])
      }
    }
    // No full build here: the implementer runs the narrow check its change
    // needs, and the fast command runs on the main worktree after the merge
    // (the Check phase). A broken story is caught there and becomes a fix story.
    const r = (await runAgent(implPrompt(story, wt, task), { schema: IMPL_SCHEMA, label: `impl:${key}`, phase: 'Implement', model: modelFor('impl'), role: 'impl' })) || { status: 'blocked', summary: 'The implementer returned no result (it may have died).' }
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
    const paths = git(['-C', wt, 'diff', '--name-only', `${integration}...HEAD`]).split('\n').filter(Boolean)
    if (paths.length) write('files', key, paths)
    return null
  }
  // ---- the implement loop: claim a pickable story, then run its implementer ----
  const implRunning = new Map()
  const implFailed = new Set()
  const claimedKeys = new Set()
  const CONTINUE_TASK = 'A worktree of an earlier attempt is already in place, with its commits. Continue from it: do not start over, and fix every open finding above.'
  const implementOne = async (key) => {
    const wt = worktreeOf(key)
    const existing = fs.existsSync(wt)
    if (!existing) {
      dropWorktree(key)
      fs.mkdirSync(storiesDir, { recursive: true })
      // While the lane holds a merge, the integration branch carries a story
      // that may yet be sent back and reset away. A worktree cut from it would
      // carry that story's commits and replay them at its own merge, so a new
      // worktree starts from the lane's base, the last head that passed. The
      // base is recorded on the branch, and the lane rebases from it.
      const from = laneBase || git(['rev-parse', integration]).trim()
      git(['worktree', 'add', wt, '-b', `story/${key}`, from])
      git(['config', `branch.story/${key}.omnibase`, from])
      if (laneBase) log(`${key}: worktree from the lane base ${from.slice(0, 7)}, not ${integration}'s head, while the lane holds a merge`)
    }
    const bad = await runImplementer(key, wt, readStory(key), existing ? CONTINUE_TASK : '')
    if (bad) return bad
    write('move', key, ['--to', 'implemented'])
    return { key, outcome: 'implemented' }
  }
  // Takes pickable stories while fewer than `implementers` run: the claim comes
  // first, the implementer after. Called on every poll tick and after each
  // implementer ends. The limit counts distinct stories claimed in this run; a
  // story already claimed once may be claimed again after it comes back.
  const implementLoop = () => {
    while (implRunning.size < implementers) {
      const keys = omni(['story', 'list', '--pickable']).split('\n').map((l) => l.trim().split(/\s+/)[0]).filter(Boolean)
      const key = keys.find((k) => !implRunning.has(k) && !implFailed.has(k) && !clarityFailed.has(k) && (claimedKeys.has(k) || claimedKeys.size < storyLimit))
      if (!key) break
      try {
        ensureFast()
        write('claim', key, ['--stage', 'implementing'])
      } catch (e) {
        implFailed.add(key)
        outcomes.push({ key, outcome: 'failed', error: String(e && e.message || e) })
        continue
      }
      claimedKeys.add(key)
      implRunning.set(key, implementOne(key).catch((e) => {
        implFailed.add(key)
        try { write('release', key) } catch {}
        return { key, outcome: 'failed', error: String(e && e.message || e) }
      }).then((o) => {
        implRunning.delete(key)
        outcomes.push(o)
        if (o.error) log(`${key}: ${o.error}`)
        pull()
      }))
    }
  }

  // ---- review definitions (used by the review loop) ------
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
- A new test, test file, test helper, fixture or snapshot in the diff is a finding: the implementer writes no tests, the lane's test author does. An existing test updated to a decision the story names is fine.
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
    let verdict = null
    for (let attempt = 0; attempt < 2 && !verdict; attempt++) {
      verdict = await runAgent(reviewPrompt(readStory(key), wt), { schema: REVIEW_SCHEMA, label: `review:${key}`, phase: 'Review', model: modelFor('review'), role: 'review' })
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
      write('move', key, ['--to', 'reviewed'])
      return { key, outcome: 'reviewed' }
    }
    // A finding sends the story back to the backlog: a fresh implementer and a
    // fresh reviewer take the next attempt (Rework from the backlog, D1, D4).
    return sendBack(key, 'review', verdict.findings || [])
  }

  // ---- merge: one worker on the main worktree ------------------------------
  const ranges = new Map()
  const doneKeys = []
  const returns = []
  const MAX_ATTEMPTS = 3
  // The attempts a story has used: each failed attempt's findings carry its number.
  const attemptOf = (story) => (story.findings || []).reduce((n, f) => Math.max(n, Number((String(f.text).match(/^\[attempt (\d+)\]/) || [])[1] || 0)), 0)
  // Returns a story to the backlog with the findings of this attempt on it and
  // its priority 0. Its worktree and branch stay, so the next pick continues
  // from them with fresh agents. After MAX_ATTEMPTS failed
  // attempts the story is blocked in the backlog with a thread listing every
  // attempt's findings. A rebase conflict (`counts` false) returns the story
  // without using an attempt.
  const sendBack = (key, stage, findings, counts = true) => {
    const story = readStory(key)
    const attempt = attemptOf(story) + (counts ? 1 : 0)
    const prefix = counts ? `[attempt ${attempt}] ` : ''
    for (const f of findings) write('finding', key, ['--stage', stage, '--text', `${prefix}${f.text}`, ...(f.files || []).flatMap((p) => ['--file', p])])
    if (story.priority !== 0) omni(['story', 'edit', key, '--priority', '0'])
    write('move', key, ['--to', 'backlog'])
    if (counts && attempt >= MAX_ATTEMPTS) {
      const all = (readStory(key).findings || []).filter((f) => /^\[attempt /.test(String(f.text))).map((f) => `- ${f.text}${(f.files || []).length ? ` (${f.files.join(', ')})` : ''}`).join('\n')
      const id = openThread(`${key}: still failing after ${attempt} attempts`, `The story was returned to the backlog ${attempt} times. The findings of every attempt:\n\n${all}\n`)
      linkAndBlock(key, [id])
      returns.push({ key, stage, attempt, blocked: true, thread: id })
      return { key, outcome: 'attempts_exhausted', thread: id }
    }
    returns.push({ key, stage, attempt, blocked: false })
    return { key, outcome: 'returned', attempt }
  }
  const limitReached = () => doneKeys.length >= storyLimit
  const moveDone = (key) => {
    write('move', key, ['--to', 'done'])
    doneKeys.push(key)
  }
  const mergeOutcomes = []
  const REBASE_FINDING = 'rebase conflict in '
  // Rebases the story in its worktree, then fast-forwards the integration
  // branch. Returns null when it merged, else the files that conflict.
  // The commit a story's branch was cut from: recorded at the cut (US worktrees
  // of this run), else git's fork point from the integration branch's reflog,
  // else the plain merge base. Rebasing from it replays only the story's own
  // commits, never a merge the integration branch has since reset away.
  const forkPointOf = (key) => {
    const isAncestor = (a, b) => tryRun('git', ['-C', root, 'merge-base', '--is-ancestor', a, b]) !== null
    const tip = (tryRun('git', ['-C', root, 'rev-parse', `story/${key}`]) || '').trim()
    const recorded = (tryRun('git', ['-C', root, 'config', '--get', `branch.story/${key}.omnibase`]) || '').trim()
    // The recorded base holds only while the branch still sits on it. An
    // implementer that rebased the branch onto the integration branch after a
    // conflict left the base behind: while the base is still on the integration
    // branch, the merge base is the newer of the two and replays only the
    // story's commits (it is the base itself when nothing moved). A base the
    // integration branch reset away is kept, since the merge base would lie
    // below it. A base that is not on the branch at all means the branch was
    // rebuilt elsewhere, and the fallbacks below decide.
    if (recorded && tryRun('git', ['-C', root, 'rev-parse', '--verify', '--quiet', `${recorded}^{commit}`]) !== null && isAncestor(recorded, tip)) {
      if (!isAncestor(recorded, integration)) return recorded
      return (tryRun('git', ['-C', root, 'merge-base', integration, tip]) || '').trim() || recorded
    }
    const fork = (tryRun('git', ['-C', root, 'merge-base', '--fork-point', integration, `story/${key}`]) || '').trim()
    // A branch the lane merged before and reset away has its own tip in the
    // integration branch's reflog, so the fork point is the tip and a rebase
    // from it would replay nothing: fall back to the merge base, which may
    // replay another story's reset-away commits, and say so.
    if (fork && fork !== tip) return fork
    const base = (tryRun('git', ['-C', root, 'merge-base', integration, `story/${key}`]) || '').trim()
    if (fork && base) log(`${key}: its branch was on ${integration} before, so git's fork point is its own tip; rebasing from the merge base ${base.slice(0, 7)}, which may replay commits of other stories`)
    return base
  }
  const tryMerge = (key) => {
    const wt = worktreeOf(key)
    const base = git(['rev-parse', 'HEAD']).trim()
    const fork = forkPointOf(key)
    try { git(['-C', wt, ...(fork ? ['rebase', '--onto', integration, fork] : ['rebase', integration])]) } catch (e) {
      const files = (tryRun('git', ['-C', wt, 'diff', '--name-only', '--diff-filter=U']) || '').split('\n').filter(Boolean)
      tryRun('git', ['-C', wt, 'rebase', '--abort'])
      return files.length ? files : ['(unknown)']
    }
    git(['merge', '--ff-only', `story/${key}`])
    // The branch now sits on this head: a later rebase (after a send-back)
    // starts from here and replays only the story's own commits.
    git(['config', `branch.story/${key}.omnibase`, base])
    ranges.set(key, { base, head: git(['rev-parse', 'HEAD']).trim() })
    const fresh = readStory(key)
    ;(fresh.findings || []).forEach((f, i) => {
      if (f.stage === 'testing' && !f.resolved && String(f.text).startsWith(REBASE_FINDING)) write('resolve', key, ['--finding', String(i)])
    })
    return null
  }
  // ---- check: exercise a merged story, and the fix story when it fails ------
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
    return `You check ONE merged story of the feature "${feature}" by running it. Its branch is already merged into ${integration} on the main worktree ${root}, and the stack is up: the one \`${stackUp || '(no stack_up configured)'}\` starts, which ran for this batch. Exercise the story against that stack, not another copy on the machine. You change nothing: no edits, no commits.

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
- The stack has no users. For a credential run \`make test-stack-seed NAME=<story key, lower case>\` in ${root}: it prints a workspace id, a session token (the cookie \`omni_session=<token>\` for human routes and the app) and an API key secret (\`Authorization: Bearer <secret>\` for agent routes and the omni CLI with OMNI_SERVER set to the stack). Never write into the stack's database yourself.
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
      verdict = await runAgent(checkPrompt(story), { schema: CHECK_SCHEMA, label: `verify:${key}${attempt ? ' retry' : ''}`, phase: 'Test lane', model: modelFor('verify'), role: 'verify' })
    }
    if (!verdict) return null
    const failures = []
    ;(story.criteria || []).forEach((c, i) => {
      const r = (verdict.criteria || []).find((x) => x.index === i)
      if (!r || !r.ok) failures.push({ criterion: c.text || c, text: `${c.text || c}: ${r ? r.evidence : 'not exercised'}` })
    })
    return { story, failures, verdict }
  }
  // ---- tests: the lasting regression tests of a checked story -------------------
  // Same author, reviewer and cut steps as ralph-spec, run on the main worktree
  // inside the test lane, so nothing else touches it meanwhile.
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
      questions: { type: 'array', items: { type: 'string' }, description: 'With status needs_decision: what the story does not settle' },
      commit: { type: 'string', description: 'The test: commit hash, or empty string when no test change was needed' },
      check: { type: 'string', description: 'The one command, run from the repo root, that runs exactly the tests you added (e.g. cargo test -p omni-core --test design_tokens, or pnpm --filter web exec vitest run src/x.test.tsx); empty when you added nothing' },
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

  const WRAP_SCHEMA = {
    type: 'object',
    required: ['healthy', 'journeys', 'report'],
    properties: {
      healthy: { type: 'boolean', description: 'Full typecheck + suite pass and every cross-story journey worked' },
      journeys: { type: 'array', items: { type: 'string' }, description: 'Each cross-story journey you exercised, with the outcome' },
      report: { type: 'string' },
    },
  }

  const bullets = (items) => items.map((i) => `- ${i}`).join('\n')
  const testReports = []
  const testsSoFar = () => testReports.flatMap((t) => t.added.map((a) => `[${t.key}] ${a.test} (${a.file})`))
  const authorPrompt = (story, base, criteria) => `You write the lasting regression tests for one story of the feature "${feature}", with a fresh context.
The implementation is merged and committed on ${integration} in the main worktree ${root}, and a verifier has confirmed it matches its criteria.
You own tests; you never change production code. Work only in ${root}.

## Story ${story.key}: ${story.title}
${story.text || ''}

### Acceptance criteria
${(story.criteria || []).map((c) => `- ${c.text || c}`).join('\n')}

### Decisions already taken (settled: test against them as if the spec said so)
${(story.decisions || []).map((d) => `- ${d.question} -> ${d.choice} (${d.by})`).join('\n') || '(none)'}

## Criteria the verifier confirmed (observed values are withheld so they cannot shape your oracle)
${bullets(criteria.map((c) => c.criterion))}
${testsSoFar().length ? `\n## Tests added earlier in this run (part of the existing suite; do not re-test what they pin)\n${bullets(testsSoFar())}\n` : ''}${CONVENTIONS}${TEST_POLICY}
## Rules
- Record \`git rev-parse HEAD\` first. You may change only ${TEST_ASSETS}.
- ORACLE FIRST (skill step 1): write the oracle table from the story text above BEFORE you open any production file or run the code. Return it in oracle. Every expected value in your tests comes from a row. If the code disagrees with a row and the story clearly supports the row, that is a defect (status found_defect); if the story does not settle it, return status needs_decision with the question in questions. In both cases leave no uncommitted test changes.
- INVENTORY (skill step 2), before writing any test. Besides the skill's search, search by the names in the story's files and by what \`git diff ${base}..HEAD\` touches. Record each existing test found in existingCoverage, and for each new test the closest existing ones in overlapCheck.
- PROVE IT GOES RED (skill step 6) for every new test: record in sensitivity each mutant you applied, that the new test went red, and which existing tests you ran against it that stayed green. Production files must be unchanged when you finish (git status shows no production changes).
- Before you commit, run the test files you touched and the typecheck of their crate or package, not the full suite: the lane ran it on this commit before you started, and runs your new tests once more after you with the command you return in check. Leave the working tree clean: no mutant left in a production file, no uncommitted test file. Commit only test files: "test: <summary>". If you add nothing, commit nothing and return an empty commit and an empty check.
Return raw structured data only.`
  // Writes the story's regression tests on the main worktree, then runs them.
  // Returns {ok: true}, {ok: false, defect} when the author found the story
  // wrong, or {ok: false, thread} when the author stopped or returned nothing.
  const testStory = async (key, criteria) => {
    phase('Test lane')
    const story = readStory(key)
    const base = (ranges.get(key) || {}).base || git(['rev-parse', 'HEAD']).trim()
    const start = git(['rev-parse', 'HEAD']).trim()
    const entry = { key, added: [], reused: [], removed: [], changed: [], commits: [] }
    testReports.push(entry)
    let result = null
    for (let attempt = 0; attempt < 2 && !result; attempt++) {
      result = await runAgent(authorPrompt(story, base, criteria), { schema: TESTS_SCHEMA, label: `tests:${key}`, phase: 'Test lane', model: modelFor('tests'), role: 'tests' })
    }
    // Whatever the author left uncommitted goes: a mutant in a production file,
    // or a test file it did not commit. Ignored files stay.
    const leftovers = (tryRun('git', ['-C', root, 'status', '--porcelain']) || '').split('\n').filter(Boolean)
    if (leftovers.length) {
      git(['checkout', '--', '.'])
      git(['clean', '-fd'])
      laneLog(key, `restored ${leftovers.length} file${leftovers.length === 1 ? '' : 's'} the test author left uncommitted`)
    }
    if (!result) {
      const id = openThread(`${key}: the test author returned nothing`, 'The test author returned no result twice. The implementation is merged and checked; only the tests are missing.\n')
      linkAndBlock(key, [id])
      return { ok: false, thread: id }
    }
    if (result.status !== 'done') {
      if (result.status === 'found_defect' && result.defect) return { ok: false, defect: `${result.defect.test}\nObserved: ${result.defect.observed}\nExpected: ${result.defect.expected}` }
      const what = (result.questions || []).join('\n') || result.blockers || result.status
      const id = openThread(`${key}: the test author stopped (${result.status})`, `${what}\n`)
      linkAndBlock(key, [id])
      return { ok: false, thread: id }
    }
    entry.added = result.added.map((t) => ({ test: t.test, file: t.file, risk: t.risk }))
    entry.reused = result.reused.map((t) => t.test)
    entry.removed = result.changedOrRemoved.filter((t) => t.action === 'removed').map((t) => t.test)
    entry.changed = result.changedOrRemoved.filter((t) => t.action === 'changed').map((t) => t.test)
    entry.commits = (tryRun('git', ['-C', root, 'log', '--format=%h %s', `${start}..HEAD`]) || '').split('\n').filter(Boolean)
    // The author's own tests, run once more by the lane. A red run drops the
    // test commits: the story is verified without them, and the report says so.
    const check = String(result.check || '').trim()
    if (entry.commits.length && check) {
      const run = await runCmd(check, root, 'check')
      if (run.ok) laneLog(key, 'tests passed')
      else {
        git(['reset', '--hard', start])
        git(['clean', '-fd'])
        entry.dropped = `\`${check}\` failed:\n${tail(run.output, 1500)}`
        entry.added = []
        entry.commits = []
        laneLog(key, `tests dropped: \`${check}\` failed`)
      }
    }
    return { ok: true }
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
  // A line the story parser would take for a label or a heading is quoted.
  const flat = (text) => String(text).split('\n').filter((l) => l.trim()).map((l, i) => (i > 0 && /^\s*(#|- \[|\*\*)/.test(l) ? `> ${l}` : l)).join('\n')
  const docLinks = (story) => (story.links || []).filter((l) => l.doc_id).map((l) => `${l.doc_id}:${l.node_id}:${l.label || ''}`)
  // ---- the loop: implementers, reviewers and the test lane together -----
  // Each pass starts what can start; a finished stage wakes the next pass. A
  // story's done can make others pickable, so the loop also wakes on that.
  const reviewing = new Map()
  const reviewOutcomes = []
  const reviewFailed = new Set()
  const numberOf = (s) => (Number.isInteger(s.number) ? s.number : Number(String(s.key).replace(/\D/g, '')))
  // Claims implemented stories (priority, then number) while fewer than
  // `reviewers` run: the claim first, the reviewer after. Called on every poll
  // tick and after each reviewer ends.
  const reviewLoop = () => {
    while (reviewing.size < reviewers) {
      const queue = listStories().filter((s) => s.status === 'implemented' && !s.blocked && !reviewing.has(s.key) && !reviewFailed.has(s.key))
        .sort((a, b) => (a.priority ?? 3) - (b.priority ?? 3) || numberOf(a) - numberOf(b))
      if (!queue.length) break
      const { key } = queue[0]
      try {
        ensureFast()
        write('claim', key, ['--stage', 'review'])
      } catch (e) {
        reviewFailed.add(key)
        reviewOutcomes.push({ key, outcome: 'failed', error: String(e && e.message || e) })
        continue
      }
      phase('Review')
      reviewing.set(key, review(key).catch((e) => {
        reviewFailed.add(key)
        try { write('release', key) } catch {}
        return { key, outcome: 'failed', error: String(e && e.message || e) }
      }).then((o) => {
        reviewing.delete(key)
        reviewOutcomes.push(o)
        if (o.error) log(`${key}: ${o.error}`)
        pull()
      }))
    }
  }
  // ---- the test lane: one story at a time on the main worktree --------------
  // Claims the first reviewed story (priority, then number), merges it, runs the
  // fast command, the stack and the criteria, then its regression tests. The
  // first failure puts the integration branch back at the lane's base and
  // sends the story back to the backlog. Called on every poll tick and after
  // the lane ends.
  let laneRunning = null
  // The integration branch's head when the lane claimed its story: new
  // worktrees start from it while the lane holds a merge (see implementOne).
  let laneBase = null
  const laneFailed = new Set()
  const tail = (text, max = 4000) => String(text).slice(-max)
  // The host's view of the main worktree the moment a stack command fails: HEAD,
  // the tree, each path the output reports missing (a container path is looked
  // up under the main worktree too, past its mount point) and the compose
  // containers with the folder each was created from. Captured before the lane
  // resets the branch, so the finding shows what the container should have seen.
  const MISSING_LINE = /No such file or directory|couldn't read|failed to read/i
  const pathsIn = (line) => [...new Set((line.match(/[^\s'"`()[\]]+/g) || [])
    .map((t) => t.replace(/^[`'"(]+|[`'",;:.)]+$/g, ''))
    .filter((t) => (t.includes('/') && t.length > 1 && !/^[a-z]+:\/\//.test(t)) || /^[\w@+-]+\.\w+$/.test(t)))]
  const onHost = (token) => {
    const parts = token.split('/').filter(Boolean)
    const candidates = path.isAbsolute(token) ? [token, ...(parts.length > 1 ? [path.join(root, ...parts.slice(1))] : [])] : [path.join(root, token)]
    for (const c of candidates) {
      try {
        const st = fs.statSync(c)
        return `${token}: present on the host at ${c} (${st.isDirectory() ? 'a directory' : `${st.size} bytes`})`
      } catch {}
    }
    return `${token}: missing on the host (looked at ${candidates.join(', ')})`
  }
  // The compose project the stack command runs against, as its -p,
  // --project-name or COMPOSE_PROJECT_NAME names it; none for the default.
  const composeProject = () => {
    const m = /(?:^|\s)(?:-p|--project-name)[\s=]+(\S+)|(?:^|\s)COMPOSE_PROJECT_NAME=(\S+)/.exec(stackCmd || '')
    return m ? ['-p', m[1] || m[2]] : []
  }
  const composeState = () => {
    const ps = tryRun('docker', ['compose', ...composeProject(), 'ps', '--format', '{{.Name}} {{.Status}}'], { cwd: root, timeout: 15000 })
    if (ps === null) return 'not available'
    const rows = ps.split('\n').map((l) => l.trim()).filter(Boolean)
    if (!rows.length) return 'no container'
    return rows.map((row) => {
      const dir = tryRun('docker', ['inspect', '--format', '{{index .Config.Labels "com.docker.compose.project.working_dir"}}', row.split(/\s+/)[0]], { cwd: root, timeout: 15000 })
      return `${row}, created from ${dir === null ? '(unknown)' : dir.trim() || '(no working_dir label)'}`
    }).join('\n')
  }
  const hostEvidence = (output) => {
    const status = (tryRun('git', ['-C', root, 'status', '--short']) || '').trim()
    const missing = [...new Set(String(output).split('\n').filter((l) => MISSING_LINE.test(l)).flatMap(pathsIn))]
    return [
      `- HEAD: ${(tryRun('git', ['-C', root, 'rev-parse', 'HEAD']) || '(unknown)').trim()}`,
      `- git status --short: ${status ? `\n${status}` : 'clean'}`,
      `- paths the output reports missing: ${missing.length ? `\n${missing.map((p) => `  ${onHost(p)}`).join('\n')}` : 'none'}`,
      `- docker compose ps: ${composeState()}`,
    ].join('\n')
  }
  // The finding of a failed stack command: its output, cut first so the host
  // evidence after it fits under the cap whole.
  const stackFinding = (what, result) => {
    const evidence = `Host at that moment:\n${hostEvidence(result.output)}`.slice(0, 2000)
    return { text: `${what} Output:\n\`\`\`\n${tail(result.output, 4000 - evidence.length)}\n\`\`\`\n${evidence}`, files: [] }
  }
  const undoLane = (base) => {
    git(['reset', '--hard', base])
    fs.rmSync(LANE_FILE, { force: true })
  }
  const failLane = async (key, base, findings, reason) => {
    undoLane(base)
    const o = sendBack(key, 'testing', findings)
    checkOutcomes.push({ key, outcome: o.outcome, failures: findings.length, thread: o.thread })
    laneLog(key, `sent back: ${reason}`)
    await rebuildStack(key, base)
  }
  const laneLog = (key, message) => log(`lane:${key}: ${message}`)
  // True while the lane runs stack_up or the stack command, which restart the
  // server behind omni: the loops read it when a poll fails.
  let stackBusy = false
  const runStack = async (cmd, name) => {
    stackBusy = true
    try { return await runCmd(cmd, root, name) } finally { stackBusy = false }
  }
  // True once this lane story ran stack_up: the stack then holds the story's
  // build, and a lane that fails after that rebuilds it from the reset tree,
  // so the stack is left on the integration branch's head, not on a story
  // that was sent back.
  let stackUpRan = false
  const rebuildStack = async (key, base) => {
    if (!stackUpRan) return
    laneLog(key, `rebuilding the stack from ${integration} at ${base.slice(0, 7)}`)
    const up = await runStack(stackUp, 'stack_up')
    laneLog(key, up.ok ? `stack rebuilt from ${integration} at ${base.slice(0, 7)}` : `the stack did not rebuild: \`${stackUp}\` exited non-zero\n${tail(up.output, 1000)}`)
  }
  // The story's branch and worktree, which an older run may have dropped after
  // merging the story: nothing is left to merge, so the story goes back for a
  // fresh attempt without an attempt counted.
  const branchMissing = (key) => tryRun('git', ['-C', root, 'rev-parse', '--verify', '--quiet', `refs/heads/story/${key}`]) === null || !fs.existsSync(worktreeOf(key))
  const laneStory = async (key) => {
    phase('Test lane')
    const base = git(['rev-parse', 'HEAD']).trim()
    laneBase = base
    stackUpRan = false
    laneLog(key, `claimed (${integration} at ${base.slice(0, 7)})`)
    if (branchMissing(key)) {
      sendBack(key, 'testing', [{ text: `branch story/${key} is missing: nothing to merge onto ${integration}; start again from ${integration}`, files: [] }], false)
      mergeOutcomes.push({ key, outcome: 'branch_missing' })
      laneLog(key, `sent back: branch story/${key} is missing`)
      return
    }
    fs.mkdirSync(path.dirname(LANE_FILE), { recursive: true })
    fs.writeFileSync(LANE_FILE, JSON.stringify({ key, base }) + '\n')
    const conflict = tryMerge(key)
    if (conflict) {
      fs.rmSync(LANE_FILE, { force: true })
      sendBack(key, 'testing', [{ text: `${REBASE_FINDING}${conflict.join(', ')}: the branch no longer rebases onto ${integration}; rebase it onto ${integration} in its worktree and resolve the conflicts`, files: conflict }], false)
      mergeOutcomes.push({ key, outcome: 'rebase_conflict', files: conflict })
      laneLog(key, `sent back: rebase conflict in ${conflict.join(', ')}`)
      return
    }
    mergeOutcomes.push({ key, outcome: 'merged' })
    laneLog(key, `merged onto ${integration}`)
    // The fast command runs first and alone: the stack's web image copies the
    // renderer build the fast command produces, so a stack built at the same
    // time served a stale renderer and the verifier checked the wrong build.
    const fastResult = await runFast(root)
    if (!fastResult.ok) return failLane(key, base, [{ text: `The fast command (\`${fast}\`) failed on the main worktree. Output:\n\`\`\`\n${tail(fastResult.output)}\n\`\`\``, files: [] }], 'the fast command failed')
    laneLog(key, 'fast passed')
    stackUpRan = true
    const up = await runStack(stackUp, 'stack_up')
    if (!up.ok) return failLane(key, base, [stackFinding(`The stack did not start: \`${stackUp}\` exited non-zero.`, up)], 'the stack did not start')
    const stackResult = await runStack(stackCmd, 'stack')
    if (!stackResult.ok) return failLane(key, base, [stackFinding(`The stack command (\`${stackCmd}\`) failed on the main worktree.`, stackResult)], 'the stack command failed')
    laneLog(key, 'stack passed')
    const r = await exercise(key)
    if (!r) {
      undoLane(base)
      const id = openThread(`${key}: the verifier returned nothing`, 'The verifier returned no result twice.\n')
      linkAndBlock(key, [id])
      checkOutcomes.push({ key, outcome: 'unchecked', thread: id })
      laneLog(key, 'blocked: the verifier returned nothing')
      await rebuildStack(key, base)
      return
    }
    if (r.failures.length > 0) return failLane(key, base, r.failures.map((f) => ({ text: f.text, files: [] })), `${r.failures.length} criteri${r.failures.length === 1 ? 'on' : 'a'} failed`)
    laneLog(key, 'criteria passed')
    const merged = git(['rev-parse', 'HEAD']).trim()
    const tested = await testStory(key, r.verdict.criteria || [])
    if (tested.defect) return failLane(key, base, [{ text: tested.defect, files: [] }], 'the test author found a defect')
    if (tested.ok) {
      passCheck(key, r.story)
      moveDone(key)
      laneLog(key, 'done')
    } else {
      // The author stopped: drop whatever it committed; ignored files stay.
      git(['reset', '--hard', merged])
      git(['clean', '-fd'])
      laneLog(key, 'blocked: the test author stopped')
    }
    fs.rmSync(LANE_FILE, { force: true })
    checkOutcomes.push(tested.ok ? { key, outcome: 'passed' } : { key, outcome: 'tests_blocked', thread: tested.thread })
  }
  const testLoop = () => {
    if (laneRunning) return
    const queue = listStories().filter((s) => s.status === 'reviewed' && !s.blocked && !laneFailed.has(s.key))
      .sort((a, b) => (a.priority ?? 3) - (b.priority ?? 3) || numberOf(a) - numberOf(b))
    if (!queue.length) return
    const { key } = queue[0]
    try {
      ensureFast()
      write('claim', key, ['--stage', 'testing'])
    } catch (e) {
      laneFailed.add(key)
      checkOutcomes.push({ key, outcome: 'failed', error: String(e && e.message || e) })
      return
    }
    laneRunning = laneStory(key).catch((e) => {
      laneFailed.add(key)
      checkOutcomes.push({ key, outcome: 'failed', error: String(e && e.message || e) })
      log(`${key}: ${e && e.message || e}`)
      try { applyLane() } catch {}
      try { write('release', key) } catch {}
    }).then(() => {
      laneRunning = null
      laneBase = null
      pull()
    })
  }
  // ---- the supervisor: the loops share `stop`, set when nothing is left to pull
  // Each loop polls on every tick and right after one of its own agents ends.
  let stop = false
  // A poll that cannot reach the server while the lane's stack restarts it is
  // logged once, as a pause, and not again until a poll succeeds; any other
  // failure is logged as it comes.
  let pollsPaused = false
  const CONNECTION_ERROR = /Connection refused|http_50[23]|cannot reach the server/i
  const pollError = (where, e) => {
    const message = String(e && e.message || e)
    if (stackBusy && CONNECTION_ERROR.test(message)) {
      if (!pollsPaused) log('lane: stack restarting, polls paused')
      pollsPaused = true
      return
    }
    log(`${where}: ${message}`)
  }
  const pull = () => {
    if (stop) return
    try { implementLoop(); reviewLoop(); testLoop(); pollsPaused = false } catch (e) { pollError('pull', e) }
  }
  // True when a story can still be claimed or this run still holds one.
  const claimableOrHeld = () => {
    if (implRunning.size > 0 || reviewing.size > 0 || laneRunning) return true
    const all = listStories()
    if (all.some((s) => s.holder && s.holder.run === runId && ['implementing', 'review', 'testing'].includes(s.status))) return true
    if (all.some((s) => s.status === 'implemented' && !s.blocked && !reviewFailed.has(s.key))) return true
    if (all.some((s) => s.status === 'reviewed' && !s.blocked && !laneFailed.has(s.key))) return true
    return omni(['story', 'list', '--pickable']).split('\n').map((l) => l.trim().split(/\s+/)[0]).filter(Boolean)
      .some((k) => !implFailed.has(k) && !clarityFailed.has(k) && (claimedKeys.has(k) || claimedKeys.size < storyLimit))
  }
  while (!stop) {
    try { unblockPass(); pollsPaused = false } catch (e) { pollError('unblock', e) }
    pull()
    if (!claimableOrHeld()) { stop = true; break }
    await Promise.race([...implRunning.values(), ...reviewing.values(), laneRunning, new Promise((r) => setTimeout(r, POLL_MS))].filter(Boolean))
  }

  // ---- report: release what is left, journeys, the post on the Run thread ---
  phase('Report')
  for (const s of listStories()) {
    if (!s.holder || s.holder.run !== runId || s.status === 'done' || s.status === 'cancelled') continue
    try { write('release', s.key) } catch (e) { log(`${s.key}: release failed: ${e && e.message || e}`) }
  }
  const WRAP_PROMPT = `Final check after a delivery run on the feature "${feature}", on the main worktree ${root} (branch ${integration}).
Stories done in this run: ${doneKeys.map((k) => `${k} ${(listStories().find((s) => s.key === k) || {}).title || ''}`).join('; ')}.
1. Run the fast command${fast ? ` \`${fast}\`` : ''} in ${root}.
2. Each story was checked on its own. Find the user journeys in the feature's docs (omni doc list --feature ${feature}, omni doc read) and in these stories (omni story read <key>) that span several of them (one story's output is another's input, a flow through several screens or endpoints) and exercise up to 3 of them for real, with the real components${stackCmd ? `, using the stack command \`${stackCmd}\`` : ''}. Report each with its outcome.
3. Stop every process and remove every container you started for the journeys, and confirm none is left.
Do NOT change any code. healthy=false if the fast command fails or a journey fails; a failure is reported, not repaired.
Return raw structured data only.`
  let wrap = null
  if (doneKeys.length > 0) {
    ensureFast()
    wrap = await runAgent(WRAP_PROMPT, { schema: WRAP_SCHEMA, label: 'final check', phase: 'Report', model: modelFor('wrap'), role: 'wrap' })
  }
  const final = listStories()
  const byRef = new Map()
  for (const s of final) { byRef.set(s.key, s); if (s.id) byRef.set(s.id, s) }
  const unfinished = (s) => s.status !== 'done' && s.status !== 'cancelled'
  const threadsOf = (s) => (s.links || []).map((l) => l.thread_id).filter(Boolean)
  const titleOf = (k) => (byRef.get(k) || {}).title || ''
  const lines = (items) => (items.length ? items.map((i) => `- ${i}`).join('\n') : '- none')
  const doneLines = doneKeys.map((k) => {
    const range = ranges.get(k)
    const feat = range ? (tryRun('git', ['-C', root, 'log', '--format=%h %s', `${range.base}..${range.head}`]) || '').split('\n').filter(Boolean) : []
    const tests = (testReports.find((t) => t.key === k) || { commits: [] }).commits
    return `${k} ${titleOf(k)}: ${[...feat.map((c) => `feat ${c}`), ...tests.map((c) => `test ${c}`)].join('; ') || 'no commits'}`
  })
  const blockedLines = final.filter((s) => unfinished(s) && s.blocked).map((s) => `${s.key} ${s.title}: thread ${threadsOf(s).join(', ') || '(none linked)'}`)
  const waitingLines = final.filter((s) => unfinished(s) && !s.blocked && (s.blocked_by || []).some((b) => byRef.has(b) && unfinished(byRef.get(b)))).map((s) => `${s.key} ${s.title}: waiting on ${(s.blocked_by || []).filter((b) => byRef.has(b) && unfinished(byRef.get(b))).map((b) => byRef.get(b).key).join(', ')}`)
  const remainingLines = final.filter(unfinished).map((s) => `${s.key} ${s.title}: ${s.blocked ? `${s.status}, blocked, thread ${threadsOf(s).join(', ') || '(none linked)'}` : s.status}`)
  const humanLines = final.flatMap((s) => (s.criteria || []).filter((c) => c.pending).map((c) => `${s.key}: ${c.text || c}`))
  const testLines = testReports.map((t) => `${t.key}: ${t.added.length} added${t.added.length ? ` (${t.added.map((a) => a.test).join('; ')})` : ''}, ${t.reused.length} reused, ${t.removed.length} removed${t.removed.length ? ` (${t.removed.join('; ')})` : ''}${t.dropped ? `; tests dropped: ${t.dropped.split('\n')[0]}` : ''}`)
  const journeyLines = !doneKeys.length ? ['no story was done in this run, so no journeys were run'] : !wrap ? ['the final check returned nothing'] : [`${wrap.healthy ? 'healthy' : 'NOT healthy'}: ${wrap.report || ''}`.trim(), ...(wrap.journeys || [])]
  const limitLine = storyLimit !== Infinity && limitReached() ? `The run stopped at storyLimit ${storyLimit}: ${doneKeys.length} stor${doneKeys.length === 1 ? 'y' : 'ies'} done in this run. ` : ''
  const report = `# Run ${runId} on ${feature}

## Done
${lines(doneLines)}

## Blocked
${lines(blockedLines)}

## Waiting on dependencies
${lines(waitingLines)}

## Returned to the backlog
${lines(returns.map((r) => `${r.key}: attempt ${r.attempt} failed at ${r.stage}${r.blocked ? `, blocked with thread ${r.thread}` : ', picked again with a raised priority'}`))}

## Pending human checks
${lines(humanLines)}
${released.length ? `\n## Released from a dead run\n${lines(released.map((r) => `${r.key}: ${r.status}`))}\n` : ''}
## Decisions taken
${lines(decisionsTaken.map((d) => `${d.key} (${d.by}): ${d.question} -> ${d.choice}`))}

## Tests
${lines(testLines)}

## Journeys
${lines(journeyLines)}

## Remaining
${limitLine}${remainingLines.length ? 'Not done:' : 'Nothing remains.'}
${remainingLines.length ? lines(remainingLines) : ''}
`
  let posted = true
  try { post(threadId, `Report of ${runId}`, report) } catch (e) {
    posted = false
    log(`The report could not be posted: ${e && e.message || e}`)
  }
  log(report)

  const pickable = omni(['story', 'list', '--pickable']).split('\n').filter(Boolean).length
  return { run: runId, feature, integration, thread: threadId, implementers, reviewers, storyLimit: storyLimit === Infinity ? null : storyLimit, released, clarity, unblocked, resumed, implemented: outcomes, reviewed: reviewOutcomes, merged: mergeOutcomes, checked: checkOutcomes, done: doneKeys, pickable, report, posted }
} finally {
  stopGroups()
  process.off('exit', stopGroups)
  fs.rmSync(lockPath, { force: true })
}
