#!/usr/bin/env node
// The host side of the native omni-deliver workflow: one subcommand per host
// step. The workflow script has no files, no clock and no processes, so every
// git and omni command runs here, along with the run's lock and its waits.
// The script reaches this helper through a courier agent, which writes the
// step's payload to a file with the Write tool and runs one of:
//
//   node ~/.claude/omni/deliver-step.mjs start --op <opId> --feature <name> [--force]
//   node ~/.claude/omni/deliver-step.mjs <step> --run <runId> --payload <root>/.omni/runs/<runId>/in/<opId>.json
//
// The courier protocol (tasks/omni-deliver-native-design.md):
// - The payload file holds JSON.stringify(obj) of the script's object, which
//   carries its opId. The helper reads it as UTF-8, removes one trailing
//   newline if there is one, parses it and deletes the file. start has no run
//   or root yet, so its object, {opId, feature, force} in that order, comes as
//   flags.
// - Every call prints exactly one line, JSON.stringify({ body, hash }) with
//   hash = fnv1a(JSON.stringify(body)), and exits 0. body.ok says whether the
//   step worked, body.error why not, and body.received is fnv1a of the payload
//   text, so the script can tell when a courier changed either side. Any other
//   exit code means the helper itself crashed.
// - A step that writes records its body under its opId in
//   .omni/runs/<runId>/applied.json, and a repeat of that opId gets the
//   recorded body back without running again: a retried courier never writes
//   twice.
// - Every call with --run refreshes the heartbeat in .omni/run.lock.
//
// The steps, each with its payload and the fields its body has besides ok and
// received:
// - start, from flags: run, root, branch, tip, lane, checks, stories, released, log
// - list {}: stories, pickable
// - context {key}: story, decisions, attempt, findings, uncounted, docs, threads
// - claim {key, stage}, release {key}, move {key, to}, block {key} and
//   unblock {key}: key, status, blocked, holder
// - finding {key, stage, findings: [{text, files}], counts}: key, attempt
// - decision {key, question, choice, by}: key
// - files {key}: key, files
// - thread-open {title, body, key?}: thread, and key when given
// - unblock-pass {}: unblocked
// - worktree {key}: key, path, branch, created
// - sleep {seconds}: seconds
// - clarity-pending {}: keys
// - clarity-record {key}: key, hash
// Outputs stay small, since a courier repeats every byte: never doc text.

import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** 32-bit FNV-1a over the string's UTF-16 code units, as 8 lowercase hex digits. The workflow script carries its own copy, which must agree. */
export function fnv1a(text) {
  let hash = 2166136261
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 16777619)
  }
  return (hash >>> 0).toString(16).padStart(8, '0')
}

// ---- time limits -----------------------------------------------------------
// Every git and omni call has a time limit: a synchronous `omni post` once held
// a run for 76 minutes. Past it the call is killed, and the error says it
// timed out.
const CALL_MS = 60000
// A lock whose heartbeat is older than this belongs to a run that stopped calling.
const STALE_MS = 2 * 3600000
// The longest sleep: a courier's Bash command stops after 10 minutes.
const MAX_SLEEP_S = 480
// A limit as the errors show it: minutes, or seconds below a minute.
const span = (ms) => (ms >= 60000 ? `${+(ms / 60000).toFixed(1)} min` : `${+(ms / 1000).toFixed(1)}s`)

const run = (cmd, argv, opts = {}) => {
  const timeout = opts.timeout || CALL_MS
  try {
    return execFileSync(cmd, argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...opts, timeout })
  } catch (e) {
    if (e && e.code === 'ETIMEDOUT') throw new Error(`\`${[cmd, ...argv].join(' ')}\` timed out after ${span(timeout)}`)
    throw e
  }
}
const tryRun = (cmd, argv, opts) => { try { return run(cmd, argv, opts) } catch { return null } }
// A failed call as a body's error: what omni and git print on standard error
// says what went wrong.
const failure = (e) => (String((e && e.stderr) || '').trim() || String((e && e.message) || e)).slice(0, 500)

const pad = (n) => String(n).padStart(2, '0')
const stamp = (d) => `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`
const alive = (pid) => {
  if (!Number.isInteger(pid) || pid <= 0) return false
  try { process.kill(pid, 0); return true } catch (e) { return e.code === 'EPERM' }
}
const short = (sha) => sha.slice(0, 7)

// ---- files under .omni/ ----------------------------------------------------
const readJson = (file) => { try { return JSON.parse(fs.readFileSync(file, 'utf8')) } catch { return null } }
// Written whole or not at all: parallel calls of one run read these files
// while another one replaces them.
const writeJson = (file, value) => {
  fs.mkdirSync(path.dirname(file), { recursive: true })
  const tmp = `${file}.${process.pid}.tmp`
  fs.writeFileSync(tmp, JSON.stringify(value, null, 2) + '\n')
  fs.renameSync(tmp, file)
}
// A read-modify-write of a file that parallel calls of one run share (the
// record of applied ops, the clarity cache) holds this lock, a file created
// exclusively. One left by a call that was killed in the few milliseconds it
// holds it is taken over after 10 s.
const withLock = (file, fn) => {
  const lock = `${file}.lock`
  fs.mkdirSync(path.dirname(lock), { recursive: true })
  const until = Date.now() + 30000
  for (;;) {
    try {
      fs.closeSync(fs.openSync(lock, 'wx'))
      break
    } catch (e) {
      if (e.code !== 'EEXIST') throw e
      let age = 0
      try { age = Date.now() - fs.statSync(lock).mtimeMs } catch {}
      if (age > 10000) fs.rmSync(lock, { force: true })
      else if (Date.now() > until) throw new Error(`${lock} stayed taken for 30 s`)
      else Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 20)
    }
  }
  try { return fn() } finally { fs.rmSync(lock, { force: true }) }
}
const lockPathOf = (root) => path.join(root, '.omni', 'run.lock')
const runDirOf = (root, runId) => path.join(root, '.omni', 'runs', runId)
const appliedPathOf = (root, runId) => path.join(runDirOf(root, runId), 'applied.json')
const clarityPathOf = (root) => path.join(root, '.omni', 'clarity.json')
const recordOp = (root, runId, opId, body) => withLock(appliedPathOf(root, runId), () => {
  const applied = readJson(appliedPathOf(root, runId)) || {}
  applied[opId] = body
  writeJson(appliedPathOf(root, runId), applied)
})
const recordedOp = (root, runId, opId) => {
  const applied = readJson(appliedPathOf(root, runId)) || {}
  return Object.hasOwn(applied, opId) ? applied[opId] : null
}

// ---- the lock --------------------------------------------------------------
// The lock holds the pid of the Claude Code process that runs the couriers.
// On macOS the chain is claude, then the courier's shell (/bin/zsh), then this
// node; a shell that runs its last command with exec leaves claude as the
// parent. So it is the nearest ancestor that is not a shell.
// OMNI_DELIVER_SESSION_PID sets it, for tests. null when ps cannot tell: such a
// lock goes stale only by its heartbeat.
const SHELLS = new Set(['sh', 'bash', 'zsh', 'dash', 'ksh', 'mksh', 'fish', 'tcsh', 'csh', 'ash'])
const sessionPid = () => {
  const given = process.env.OMNI_DELIVER_SESSION_PID
  if (given) return Number.isInteger(Number(given)) && Number(given) > 0 ? Number(given) : null
  let pid = process.ppid
  for (let depth = 0; depth < 10 && pid > 1; depth++) {
    const row = (tryRun('ps', ['-o', 'ppid=,comm=', '-p', String(pid)]) || '').trim().match(/^(\d+)\s+(.+)$/)
    if (!row) return null
    if (!SHELLS.has(path.basename(row[2].trim()).replace(/^-/, ''))) return pid
    pid = Number(row[1])
  }
  return null
}
// Why a lock is stale, or null while its run may still be alive: its session
// has ended, or it has made no call for 2 hours. A lock without a heartbeat
// (one written before heartbeats) is judged by its pid alone.
const staleness = (held) => {
  if (Number.isInteger(held.pid) && held.pid > 0 && !alive(held.pid)) return `its session (pid ${held.pid}) has ended`
  const beat = Date.parse(held.heartbeat)
  if (Number.isFinite(beat) && Date.now() - beat > STALE_MS) return `its last heartbeat, ${held.heartbeat}, is more than 2 hours old`
  return null
}
// Every call of the run says it is alive. A lock that another run holds now,
// after a takeover, is left as it is.
const refreshHeartbeat = (root, runId) => {
  const held = readJson(lockPathOf(root))
  if (held && held.run === runId) writeJson(lockPathOf(root), { ...held, heartbeat: new Date().toISOString() })
}
const dropLock = (root, runId) => {
  const held = readJson(lockPathOf(root))
  if (held && held.run === runId) fs.rmSync(lockPathOf(root), { force: true })
}

// ---- the board and the checkout of one run ---------------------------------
const WORKING = ['implementing', 'review', 'testing']
const CHECKS = ['fast', 'stack_up', 'stack', 'seed', 'lint']
const slugOf = (feature) => feature.toLowerCase().replace(/[^a-z0-9]/g, '-')
const boardOf = (root, runId, feature) => {
  const git = (argv, opts) => run('git', ['-C', root, ...argv], opts)
  const storiesDir = path.join(root, '..', `${path.basename(root)}-stories`)
  const branch = `deliver/${slugOf(feature)}`
  const identity = { ...process.env, OMNI_RUN: runId, OMNI_ROLE: 'deliver' }
  // Every omni call acts as this run's agent, in the feature.
  const omni = (argv, opts = {}) => run('omni', [...argv, '--feature', feature], { cwd: root, ...opts, env: identity })
  const omniJson = (argv) => {
    const parsed = JSON.parse(omni([...argv, '--json']))
    return parsed.story || parsed
  }
  const listStories = () => JSON.parse(omni(['story', 'list', '--json'])).stories
  const readStory = (key) => omniJson(['story', 'read', key])
  const write = (verb, key, extra = []) => omniJson(['story', verb, key, '--run', runId, ...extra])
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
    return write('block', key)
  }
  // A doc as `omni doc read` prints it, read once per call.
  const docs = new Map()
  const docText = (id) => {
    if (!docs.has(id)) docs.set(id, tryRun('omni', ['doc', 'read', id], { cwd: root, env: identity }) ?? `(the doc ${id} could not be read)`)
    return docs.get(id)
  }
  return { root, runId, feature, git, storiesDir, lane: path.join(storiesDir, '_lane'), branch, omni, listStories, readStory, write, openThread, linkAndBlock, docText, worktreeOf: (key) => path.join(storiesDir, key) }
}

const summary = (s) => ({ key: s.key, title: s.title, status: s.status, priority: s.priority ?? 3, blocked: !!s.blocked, holder: s.holder ? s.holder.run : null })
const statusOf = (s) => ({ key: s.key, status: s.status, blocked: !!s.blocked, holder: s.holder ? s.holder.run : null })
const pickableKeys = (b) => b.omni(['story', 'list', '--pickable']).split('\n').map((l) => l.trim().split(/\s+/)[0]).filter(Boolean)
const criterionOf = (c) => (typeof c === 'string' ? { text: c, human_check: false } : { text: c.text, human_check: !!c.human_check })
const isWorktree = (root, dir) => {
  const real = (p) => { try { return fs.realpathSync(p) } catch { return p } }
  return (tryRun('git', ['-C', root, 'worktree', 'list', '--porcelain']) || '').split('\n')
    .filter((l) => l.startsWith('worktree ')).some((l) => real(l.slice(9)) === real(dir))
}

// ---- attempts --------------------------------------------------------------
// The attempts a story has used since the human's latest answer: each failed
// attempt's findings carry its number. The unblock pass resolves every open
// finding when it records the human's answers, so only open findings count,
// and an answer restarts the budget.
const ATTEMPT = /^\[attempt (\d+)\] ?/
const attemptOf = (story) => (story.findings || []).filter((f) => !f.resolved).reduce((n, f) => Math.max(n, Number((String(f.text).match(ATTEMPT) || [])[1] || 0)), 0)
// The open findings by attempt, with their numbers taken off, and apart from
// them those that used no attempt (a rebase conflict).
const budgetOf = (story) => {
  const byAttempt = new Map()
  const uncounted = []
  for (const f of (story.findings || []).filter((x) => !x.resolved)) {
    const m = String(f.text).match(ATTEMPT)
    const item = { stage: f.stage, text: m ? String(f.text).slice(m[0].length) : String(f.text), files: f.files || [] }
    if (!m) uncounted.push(item)
    else byAttempt.set(Number(m[1]), [...(byAttempt.get(Number(m[1])) || []), item])
  }
  const findings = [...byAttempt.keys()].sort((a, c) => a - c).map((n) => ({ attempt: n, findings: byAttempt.get(n) }))
  return { attempt: attemptOf(story), findings, uncounted }
}

// ---- clarity: .omni/clarity.json -------------------------------------------
// A backlog story gets a clarity check when what the check reads changed since
// its last one: its text, its criteria, the text of its linked doc nodes and
// its decisions. The cache holds a hash of those per story key.
const VOID = new Set(['hr', 'br', 'img'])
// The element of the node with this data-omni-id in a doc as `omni doc read`
// prints it, with everything inside it. A doc in which the node is not found
// (another format, or the node is gone) counts whole.
const nodeText = (doc, id) => {
  const at = doc.indexOf(`data-omni-id="${id}"`)
  const start = at === -1 ? -1 : doc.lastIndexOf('<', at)
  const name = start === -1 ? null : (doc.slice(start).match(/^<([a-zA-Z][\w-]*)/) || [])[1]
  if (!name) return doc
  if (VOID.has(name.toLowerCase())) return doc.slice(start, doc.indexOf('>', at) + 1)
  const tags = new RegExp(`<(/?)${name}(?=[\\s/>])[^>]*>`, 'gi')
  tags.lastIndex = start
  let depth = 0
  for (let m = tags.exec(doc); m; m = tags.exec(doc)) {
    depth += m[1] ? -1 : 1
    if (depth === 0) return doc.slice(start, m.index + m[0].length)
  }
  return doc.slice(start)
}
const clarityHash = (b, story) => fnv1a(JSON.stringify({
  text: story.text || '',
  criteria: (story.criteria || []).map((c) => criterionOf(c).text),
  nodes: (story.links || []).filter((l) => l.doc_id).map((l) => nodeText(b.docText(l.doc_id), l.node_id)),
  decisions: (story.decisions || []).map((d) => [d.question, d.choice, d.by]),
}))

// The payload fields a step needs; its error names the missing ones.
const missing = (payload, ...names) => {
  const absent = names.filter((n) => typeof payload[n] !== 'string' || !payload[n])
  return absent.length ? { ok: false, error: `the payload needs ${absent.join(', ')}` } : null
}

// ---- the steps of a run ----------------------------------------------------
const STEPS = {
  list: (b) => ({ stories: b.listStories().map(summary), pickable: pickableKeys(b) }),

  // What a prompt needs about the story, and never doc text: agents read each
  // linked doc once themselves, with `omni doc read`.
  context: (b, p) => missing(p, 'key') || (() => {
    const stories = b.listStories()
    const story = stories.find((s) => s.key === p.key)
    if (!story) return { ok: false, error: `no story ${p.key} in the feature` }
    const keyOf = new Map(stories.map((s) => [s.id, s.key]))
    const docs = []
    for (const l of (story.links || []).filter((x) => x.doc_id)) {
      let doc = docs.find((d) => d.doc === l.doc_id)
      if (!doc) docs.push((doc = { doc: l.doc_id, nodes: [] }))
      doc.nodes.push({ node: l.node_id, label: l.label || '' })
    }
    return {
      story: { key: story.key, title: story.title, status: story.status, priority: story.priority ?? 3, blocked: !!story.blocked, text: story.text || '', criteria: (story.criteria || []).map(criterionOf), files_planned: story.files_planned || [], blocked_by: (story.blocked_by || []).map((id) => keyOf.get(id) || id) },
      decisions: (story.decisions || []).map((d) => ({ question: d.question, choice: d.choice, by: d.by })),
      ...budgetOf(story),
      docs,
      threads: (story.links || []).map((l) => l.thread_id).filter(Boolean),
    }
  })(),

  claim: (b, p) => missing(p, 'key', 'stage') || statusOf(b.write('claim', p.key, ['--stage', p.stage])),
  release: (b, p) => missing(p, 'key') || statusOf(b.write('release', p.key)),
  move: (b, p) => missing(p, 'key', 'to') || statusOf(b.write('move', p.key, ['--to', p.to])),
  block: (b, p) => missing(p, 'key') || statusOf(b.write('block', p.key)),
  unblock: (b, p) => missing(p, 'key') || statusOf(b.write('unblock', p.key)),

  // The findings of a failed attempt get its number, one past the attempts the
  // story has used; a failure that uses no attempt (counts false: a rebase
  // conflict, the infrastructure) gets none. An attempt that counts leaves at
  // least one finding, else the budget would not see it.
  finding: (b, p) => missing(p, 'key', 'stage') || (() => {
    const counts = p.counts !== false
    if (!Array.isArray(p.findings) || p.findings.some((f) => !f || typeof f.text !== 'string') || (counts && p.findings.length === 0)) return { ok: false, error: 'the payload needs findings: [{text, files}], at least one when the attempt counts' }
    const attempt = attemptOf(b.readStory(p.key)) + (counts ? 1 : 0)
    const prefix = counts ? `[attempt ${attempt}] ` : ''
    for (const f of p.findings) b.write('finding', p.key, ['--stage', p.stage, '--text', `${prefix}${f.text}`, ...(f.files || []).flatMap((file) => ['--file', file])])
    return { key: p.key, attempt }
  })(),

  decision: (b, p) => missing(p, 'key', 'question', 'choice', 'by') || (() => {
    b.write('decision', p.key, ['--question', p.question, '--choice', p.choice, '--by', p.by])
    return { key: p.key }
  })(),

  // What the story's branch changed since it left deliver/<slug> becomes its
  // files_changed, by which the board keeps stories on the same files apart.
  files: (b, p) => missing(p, 'key') || (() => {
    const wt = b.worktreeOf(p.key)
    if (!fs.existsSync(wt)) return { ok: false, error: `no worktree ${wt}` }
    const paths = b.git(['-C', wt, 'diff', '--name-only', `refs/heads/${b.branch}...HEAD`]).split('\n').filter(Boolean)
    if (paths.length) b.write('files', p.key, paths)
    return { key: p.key, files: paths }
  })(),

  'thread-open': (b, p) => missing(p, 'title', 'body') || (() => {
    const thread = b.openThread(p.title, p.body)
    if (!p.key) return { thread }
    try { b.linkAndBlock(p.key, [thread]) } catch (e) { return { ok: false, thread, key: p.key, error: failure(e) } }
    return { thread, key: p.key }
  })(),

  // A blocked story whose linked threads all have a conclusion gets each
  // conclusion as a human decision and is unblocked: a thread's conclusion is
  // the human's answer. The answer restarts the story's attempt budget: its
  // open findings are resolved, and attempts count from there (attemptOf).
  // A failure stops the pass with the keys it unblocked so far; the stories
  // after it are tried again by the next pass.
  'unblock-pass': (b) => {
    const unblocked = []
    try {
      for (const story of b.listStories()) {
        if (!story.blocked || story.status === 'done' || story.status === 'cancelled') continue
        const ids = (story.links || []).map((l) => l.thread_id).filter(Boolean)
        if (ids.length === 0) continue
        const threads = ids.map((id) => {
          const parsed = JSON.parse(b.omni(['thread', 'read', id, '--json']))
          return parsed.thread || parsed
        })
        if (threads.some((t) => !t.conclusion)) continue
        const recorded = (story.decisions || []).filter((d) => d.by === 'human')
        for (const t of threads) {
          if (recorded.some((d) => d.question === t.question && d.choice === t.conclusion)) continue
          b.write('decision', story.key, ['--question', t.question, '--choice', t.conclusion, '--by', 'human'])
        }
        ;(story.findings || []).forEach((f, i) => { if (!f.resolved) b.write('resolve', story.key, ['--finding', String(i)]) })
        b.write('unblock', story.key)
        unblocked.push(story.key)
      }
    } catch (e) {
      return { ok: false, unblocked, error: failure(e) }
    }
    return { unblocked }
  },

  // A new worktree starts at the tip of deliver/<slug>, what the lane last
  // landed; an existing one is kept, with what earlier attempts committed and
  // left in it.
  worktree: (b, p) => missing(p, 'key') || (() => {
    const wt = b.worktreeOf(p.key)
    const branch = `story/${p.key}`
    if (fs.existsSync(wt)) return { key: p.key, path: wt, branch, created: false }
    // A branch left without its worktree goes, so the new one starts clean.
    tryRun('git', ['-C', b.root, 'worktree', 'prune'])
    tryRun('git', ['-C', b.root, 'branch', '-D', branch])
    fs.mkdirSync(b.storiesDir, { recursive: true })
    b.git(['worktree', 'add', wt, '-b', branch, b.git(['rev-parse', `refs/heads/${b.branch}`]).trim()])
    return { key: p.key, path: wt, branch, created: true }
  })(),

  // The wait between retries: the script has no clock.
  sleep: async (b, p) => {
    const seconds = Math.min(Math.max(Number(p.seconds) || 0, 0), MAX_SLEEP_S)
    await new Promise((resolve) => setTimeout(resolve, seconds * 1000))
    return { seconds }
  },

  'clarity-pending': (b) => {
    const seen = readJson(clarityPathOf(b.root)) || {}
    return { keys: b.listStories().filter((s) => s.status === 'backlog' && !s.blocked && seen[s.key] !== clarityHash(b, s)).map((s) => s.key) }
  },
  // Recorded after the clarity check's own writes (its decisions, its
  // threads), so that they do not make the story pending again.
  'clarity-record': (b, p) => missing(p, 'key') || (() => {
    const hash = clarityHash(b, b.readStory(p.key))
    withLock(clarityPathOf(b.root), () => {
      const seen = readJson(clarityPathOf(b.root)) || {}
      seen[p.key] = hash
      writeJson(clarityPathOf(b.root), seen)
    })
    return { key: p.key, hash }
  })(),
}
// The steps whose bodies are recorded under their opId, so that a repeat does
// not write twice. The others only read, or wait.
const WRITES = new Set(['claim', 'release', 'move', 'block', 'unblock', 'finding', 'decision', 'files', 'thread-open', 'unblock-pass', 'worktree', 'clarity-record'])

// ---- start: the lock, reconcile, the deliver branch, the checks ------------
const start = (flags) => {
  const { op: opId, feature } = flags
  const force = flags.force === true
  const received = fnv1a(JSON.stringify({ opId, feature, force }))
  if (!opId || !feature) return { ok: false, received, error: 'start needs --op <opId> and --feature <name>' }
  const top = tryRun('git', ['rev-parse', '--show-toplevel'])
  if (top === null) return { ok: false, received, error: `${process.cwd()} is not in a git checkout` }
  const root = top.trim()
  const session = sessionPid()
  const lockPath = lockPathOf(root)
  const log = []
  let raw = null
  try { raw = fs.readFileSync(lockPath, 'utf8') } catch {}
  const held = raw === null ? null : readJson(lockPath)
  if (raw !== null && !held) log.push('The lock file could not be read; taking it over')
  if (held) {
    // The same start again, from this session, before its run made any call:
    // its courier failed after the helper ran. Its record answers.
    const retried = session !== null && held.pid === session && held.op === opId && held.feature === feature && held.heartbeat === held.started
    const recorded = retried ? recordedOp(root, held.run, opId) : null
    if (recorded) return recorded
    const stale = retried ? 'its start, from this session, ended before it finished' : staleness(held)
    if (!stale && !force) return { ok: false, received, error: `the tree is held by run ${held.run} for feature ${held.feature}, last heard from at ${held.heartbeat || held.started}; start with force if that run is dead` }
    log.push(`Took over the lock of run ${held.run} for feature ${held.feature}: ${stale || 'start with force'}`)
  }
  const now = new Date()
  const runId = `run-${stamp(now)}-${process.pid}`
  writeJson(lockPath, { run: runId, feature, pid: session, op: opId, started: now.toISOString(), heartbeat: now.toISOString() })
  let body
  try {
    body = startRun(boardOf(root, runId, feature), log)
  } catch (e) {
    body = { ok: false, error: failure(e) }
  }
  // A start that stops gives the lock up.
  if (body.ok === false) {
    dropLock(root, runId)
    return { ok: false, received, error: body.error }
  }
  body = { ok: true, received, ...body }
  fs.mkdirSync(path.join(runDirOf(root, runId), 'in'), { recursive: true })
  writeJson(path.join(runDirOf(root, runId), 'run.json'), { run: runId, feature, root })
  recordOp(root, runId, opId, body)
  return body
}

// Everything that can stop the run comes before anything that changes the
// board or the branch.
const startRun = (b, log) => {
  const { root, git } = b
  // .omni/ holds the lock, the runs and the clarity cache, none of which is
  // committed.
  if (tryRun('git', ['-C', root, 'check-ignore', '-q', '.omni/']) === null) {
    const exclude = path.resolve(root, git(['rev-parse', '--git-path', 'info/exclude']).trim())
    let text = ''
    try { text = fs.readFileSync(exclude, 'utf8') } catch {}
    fs.mkdirSync(path.dirname(exclude), { recursive: true })
    fs.writeFileSync(exclude, `${text}${text && !text.endsWith('\n') ? '\n' : ''}.omni/\n`)
  }
  // The integration branch, checked out nowhere: stories land on it, and the
  // run never moves the checkout's own branch.
  const head = git(['rev-parse', 'HEAD']).trim()
  const was = (tryRun('git', ['-C', root, 'rev-parse', '--verify', '--quiet', `refs/heads/${b.branch}`]) || '').trim()
  const isAncestor = (a, c) => tryRun('git', ['-C', root, 'merge-base', '--is-ancestor', a, c]) !== null
  const move = !was ? 'create' : was === head ? 'same' : isAncestor(was, head) ? 'fast-forward' : isAncestor(head, was) ? 'ahead' : 'diverged'
  if (move === 'diverged') return { ok: false, error: `${b.branch} and HEAD have diverged: merge first (git merge ${b.branch}), then start again` }
  let checks
  try {
    checks = JSON.parse(b.omni(['checks', '--json']))
  } catch (e) {
    return { ok: false, error: `the checks could not be read: ${failure(e)}` }
  }
  if (fs.existsSync(b.lane) && !isWorktree(root, b.lane)) return { ok: false, error: `${b.lane} is not a worktree of ${root}, so it cannot be the lane worktree; move it away` }

  if (move === 'create') git(['update-ref', `refs/heads/${b.branch}`, head, ''])
  if (move === 'fast-forward') git(['update-ref', `refs/heads/${b.branch}`, head, was])
  const tip = move === 'create' || move === 'fast-forward' ? head : was
  log.push({
    create: `${b.branch} created at ${short(head)}, the checkout's HEAD`,
    same: `${b.branch} is at ${short(was)}, the checkout's HEAD`,
    'fast-forward': `${b.branch} fast-forwarded from ${short(was)} to ${short(head)}, the checkout's HEAD`,
    ahead: `${b.branch} kept at ${short(was)}, ahead of the checkout's HEAD ${short(head)}`,
  }[move])

  // The lane worktree stays from run to run, detached. Cleaning it never uses
  // -x: ignored build caches stay warm.
  tryRun('git', ['-C', root, 'worktree', 'prune'])
  if (fs.existsSync(b.lane)) {
    git(['-C', b.lane, 'reset', '-q', '--hard'])
    git(['-C', b.lane, 'clean', '-fdq'])
  } else {
    fs.mkdirSync(b.storiesDir, { recursive: true })
    git(['worktree', 'add', '-q', '--detach', b.lane, tip])
  }

  // A story another run holds is released: this run holds the lock, so no
  // other run of this checkout is alive. A working story nobody holds (a claim
  // whose run died before it could be released) is released as well: no run
  // can move or claim it otherwise.
  const released = []
  for (const story of b.listStories()) {
    if (story.holder ? story.holder.run === b.runId : !WORKING.includes(story.status)) continue
    released.push({ key: story.key, status: b.write('release', story.key).status })
  }
  return {
    run: b.runId,
    root,
    branch: b.branch,
    tip,
    lane: b.lane,
    checks: Object.fromEntries(CHECKS.map((name) => [name, String((checks && checks[name]) || '')])),
    stories: b.listStories().map(summary),
    released,
    log,
  }
}

// ---- a step of a run -------------------------------------------------------
// The courier writes the payload at <root>/.omni/runs/<runId>/in/<opId>.json,
// so the root comes from its path.
const PAYLOAD_PATH = /^(.+)\/\.omni\/runs\/([^/]+)\/in\/[^/]+\.json$/
const runStep = async (flags) => {
  const step = STEPS[flags.step]
  if (!step) return { ok: false, received: null, error: `no step ${flags.step}; the steps are start, ${Object.keys(STEPS).join(', ')}` }
  if (!flags.run || !flags.payload) return { ok: false, received: null, error: `${flags.step} needs --run <runId> --payload <file>` }
  const file = path.resolve(flags.payload)
  const where = file.match(PAYLOAD_PATH)
  if (!where || where[2] !== flags.run) return { ok: false, received: null, error: `the payload goes in <root>/.omni/runs/${flags.run}/in/<opId>.json, not ${file}` }
  const root = where[1]
  refreshHeartbeat(root, flags.run)
  let text
  try { text = fs.readFileSync(file, 'utf8') } catch { return { ok: false, received: null, error: `no payload at ${file}` } }
  fs.rmSync(file, { force: true })
  if (text.endsWith('\n')) text = text.slice(0, -1)
  const received = fnv1a(text)
  let payload
  try { payload = JSON.parse(text) } catch { return { ok: false, received, error: 'the payload is not JSON' } }
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return { ok: false, received, error: 'the payload is not a JSON object' }
  const info = readJson(path.join(runDirOf(root, flags.run), 'run.json'))
  if (!info) return { ok: false, received, error: `no run ${flags.run} in ${root}` }
  const writes = WRITES.has(flags.step)
  if (writes) {
    if (typeof payload.opId !== 'string' || !payload.opId) return { ok: false, received, error: `${flags.step} writes, so its payload needs an opId` }
    const recorded = recordedOp(root, flags.run, payload.opId)
    if (recorded) return recorded
  }
  let body
  try {
    body = { ok: true, received, ...(await step(boardOf(root, flags.run, info.feature), payload)) }
  } catch (e) {
    body = { ok: false, received, error: failure(e) }
  }
  if (writes) recordOp(root, flags.run, payload.opId, body)
  return body
}

const flagsOf = (argv) => {
  const flags = { step: argv[0] }
  for (let i = 1; i < argv.length; i++) {
    if (argv[i] === '--force') flags.force = true
    else if (['--run', '--payload', '--op', '--feature'].includes(argv[i])) flags[argv[i].slice(2)] = argv[++i]
    else flags.unknown = argv[i]
  }
  return flags
}

export async function main(argv = process.argv.slice(2)) {
  const flags = flagsOf(argv)
  let body
  try {
    if (flags.unknown !== undefined) body = { ok: false, received: null, error: `unknown argument ${flags.unknown}; see the header of workflows/deliver-step.mjs` }
    else body = flags.step === 'start' ? start(flags) : await runStep(flags)
  } catch (e) {
    body = { ok: false, received: null, error: failure(e) }
  }
  process.stdout.write(`${JSON.stringify({ body, hash: fnv1a(JSON.stringify(body)) })}\n`)
}

// Run as a command, also through the link at ~/.claude/omni/deliver-step.mjs:
// process.argv[1] is then the link, and import.meta.url the file it points to.
const invoked = (() => { try { return fs.realpathSync(process.argv[1]) } catch { return null } })()
if (invoked === fileURLToPath(import.meta.url)) main()
