#!/usr/bin/env node
// The host side of the native omni-deliver workflow: one subcommand per host
// step. The workflow script has no files, no clock and no processes, so every
// git and omni command runs here, along with the run's lock and its waits.
// The script reaches this helper through a courier agent, which writes the
// step's payload to a file with the Write tool and runs one of:
//
//   node ~/.claude/omni/deliver-step.mjs start --op <opId> --feature <name> [--force] --hash <hash>
//   node ~/.claude/omni/deliver-step.mjs <step> --run <runId> --payload <root>/.omni/runs/<runId>/in/<opId>.json --hash <hash>
//
// The courier protocol (tasks/omni-deliver-native-design.md):
// - The payload file holds JSON.stringify(obj) of the script's object, which
//   carries its opId. The helper reads it as UTF-8, removes one trailing
//   newline if there is one, parses it and deletes the file. start has no run
//   or root yet, so its object, {opId, feature, force} in that order, comes as
//   flags.
// - --hash is fnv1a of that payload text, as the script computed it. The
//   helper compares it with the payload it got before anything else, even
//   before it looks in the record of applied ops: a payload the courier copied
//   wrong is refused, and nothing is written or recorded.
// - Every call prints exactly one line, JSON.stringify({ body, hash }) with
//   hash = fnv1a(JSON.stringify(body)), and exits 0. body.ok says whether the
//   step worked, body.error why not, and body.received is fnv1a of the payload
//   text, so the script can tell when a courier changed either side. Any other
//   exit code means the helper itself crashed.
// - A body with ok false says whether to try the same command again: retry
//   true when that can work, after a payload that came wrong or a command that
//   failed, such as a write cut short, which the repeat carries on; retry
//   false when the repeat would be refused the same way: the board refused
//   the write (a claim of a story that is not waiting, or that another run
//   holds), another run holds the lock, a worktree on the wrong branch, a
//   payload the step cannot take.
// - A step that writes records its body under its step name and opId in
//   .omni/runs/<runId>/applied.json once it worked, and a repeat of that opId
//   gets the recorded body back without running again: a retried courier
//   never writes twice. Before it writes, the step records a pending marker
//   there. A repeat that finds the marker, after a helper that was killed or a
//   write that failed, checks the board for what the first try did and does
//   only the rest. A repeat of an opId with another payload is an error.
// - .omni/run.lock names the run that holds the checkout. Every call of that
//   run refreshes its heartbeat, and a call of a run that no longer holds it
//   fails; only a repeat is answered without it, and a repeat of a finish
//   that gave the lock up goes on without it.
// - A start repeated by the same session within 5 minutes, before its run
//   made any call, gets the first start's body back, with --force too.
// - Every step that runs records its start, its end and its payload's key in
//   .omni/runs/<runId>/timings.jsonl, and lane-prepare its rebase; finish
//   reports them.
//
// The steps, each with its payload and the fields its body has besides ok and
// received:
// - start, from flags: run, root, branch, tip, lane, thread, checks, stories
//   ({keys, counts by status, blocked}), released, log
// - checks-set {checks: {name: command}}: checks, all five
// - list {}: stories, pickable (those with a worktree from an earlier attempt
//   first, then in the board's order: priority, then how many stories each
//   one unblocks, then number)
// - context {key}: story, decisions, attempt, findings, uncounted, earlier, docs, threads
// - claim {key, stage}, release {key}, move {key, to}, block {key} and
//   unblock {key}: key, status, blocked, holder
// - finding {key, stage, findings: [{text, files}], counts}: key, attempt
// - decision {key, question, choice, by}: key
// - files {key}: key, files (how many)
// - thread-open {title, body, key?}: thread, and key when given
// - unblock-pass {}: unblocked, log
// - worktree {key}: key, path, branch, created (true when the story starts on
//   a branch of its own just cut, false when its branch has earlier work)
// - sleep {seconds}: seconds
// - clarity-pending {}: keys
// - clarity-record {key}: key, hash
// - job-start {name, key?, cmd, cwd, limitMinutes?}: id, log
// - job-wait {id, maxSeconds}: id, done, and once done result and tail
// - job-kill {id}: id, killed, done, and once done result and tail
// - lane-prepare {key}: key, outcome: ready with base and candidate; conflict
//   with files; dirty with files and count; wrong_branch with head (the
//   branch checked out, null when detached) and commits (those not on
//   story/<key>); missing or infra. Every outcome but ready and conflict has
//   a reason. dirty and wrong_branch are the story's to fix, and count as an
//   attempt; conflict does not; infra is not the story's.
// - lane-clean {}: discarded and count: what it threw away
// - lane-base {} and lane-reset {}: tip
// - lane-land {key, base}: key, landed, with base, final, files (how many)
//   and removed when landed, reason when refused; refused for a branch
//   checked out in the lane, it puts deliver/<slug> back at the base and
//   detaches the lane
// - finish {report: {outcomes: [{key, outcome, detail?}], council: {settled,
//   asked}, blocked: [{key, threads}], journeys?: [line], notes?: [line]}}:
//   thread, posted, report, stopped
// Outputs stay small, since a courier repeats every byte: never doc text, of
// a job's log only its last 2 KB, and of a list of files at most 20.
//
// The lane, story by story: lane-prepare; the checks as jobs in the lane
// worktree; the verifier; the test author; lane-clean, so that the test
// author's check runs on exactly what lands; that check; lane-land. lane-land
// refuses a lane worktree with uncommitted changes, so a check that leaves
// files git does not ignore keeps every story from landing: such files belong
// in .gitignore.
//
// Left as they are:
// - A job's command can leave members of its group running in the background
//   after its leader exited: the job ends with its leader, and nothing stops
//   them.
// - A thread whose `thread create` was cut off after the board made it stays
//   without a post: the repeat cannot know its id, and creates another.
//
// Jobs are commands that take longer than a courier's Bash call. job-start
// starts a detached supervisor (this file's hidden __supervise) and returns.
// The supervisor runs the command in a process group of its own with no
// stdin, its output going straight to .omni/runs/<runId>/jobs/<id>.log, and
// writes the result, {ok, code, signal, timedOut, durationMs, started, ended},
// to <id>.json when it ends; <id>.job.json holds the job and its pids. At its
// limit the group gets SIGTERM, then SIGKILL 10 s later. A job whose
// supervisor died without a result is lost: job-wait, job-kill and start stop
// its group the same way when ps shows that the group's leader is still the
// process the job started. A stopped job can leave behind what it started
// outside its group, such as containers in a VM: cleaning those up is the
// project's job, in its stack_up or a preflight.
// OMNI_DELIVER_KILL_GRACE_MS and OMNI_DELIVER_REBASE_MS set that grace and the
// lane's rebase limit, for tests.

import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
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
// Every timer delay in whole milliseconds, at most 2^31 - 1: setTimeout fires
// after 1 ms for anything larger, and execFileSync throws on a fraction.
const MAX_DELAY_MS = 2 ** 31 - 1
const delay = (ms) => (ms > 0 ? Math.min(Math.round(ms), MAX_DELAY_MS) : 0)
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, delay(ms)))
const nap = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, delay(ms))
// A wait in seconds, at most 480 so that it fits a courier's Bash call; 480
// when the payload gives none. Exported for its test: a wait of 480 s is
// too long for one.
export const waitSeconds = (s) => (typeof s === 'number' ? Math.min(Math.max(s, 0), MAX_SLEEP_S) : MAX_SLEEP_S)

// A long story list or doc read passes the 1 MB that execFileSync takes by default.
const MAX_BUFFER = 64 * 1024 * 1024

const run = (cmd, argv, opts = {}) => {
  const timeout = opts.timeout || CALL_MS
  try {
    return execFileSync(cmd, argv, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: MAX_BUFFER, ...opts, timeout })
  } catch (e) {
    if (e && e.code === 'ETIMEDOUT') throw Object.assign(new Error(`\`${[cmd, ...argv].join(' ')}\` timed out after ${span(timeout)}`), { timedOut: true })
    throw e
  }
}
const tryRun = (cmd, argv, opts) => { try { return run(cmd, argv, opts) } catch { return null } }
// A failed call as a body's error: what omni and git print on standard error
// says what went wrong.
const failure = (e) => (String((e && e.stderr) || '').trim() || String((e && e.message) || e)).slice(0, 500)
// Whether the same command again may work: not when the board refused the
// write for a rule a repeat meets again (a claim of a story that is not
// waiting for the stage or that another run holds, a write to a story that is
// closed); yes for any other failure, such as a timeout or a server that
// could not be reached.
const REFUSALS = /\b(not_waiting|held|not_holder|not_open|criteria_open)\b/
const retryable = (e) => !REFUSALS.test(failure(e))

const pad = (n) => String(n).padStart(2, '0')
const stamp = (d) => `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`
const alive = (pid) => {
  if (!Number.isInteger(pid) || pid <= 0) return false
  try { process.kill(pid, 0); return true } catch (e) { return e.code === 'EPERM' }
}
const short = (sha) => sha.slice(0, 7)
const real = (p) => { try { return fs.realpathSync(p) } catch { return p } }
// True when git in dir works in dir itself. A worktree folder that lost its
// .git file is not: git there finds a repository around it, and a reset or
// clean would hit that one.
const ownTree = (dir) => {
  const top = (tryRun('git', ['-C', dir, 'rev-parse', '--show-toplevel']) || '').trim()
  return top !== '' && real(top) === real(dir)
}

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
// exclusively that names its holder's pid. One left by a call that was killed
// while it held it is taken over: at once when its holder has ended, else
// once it is older than holdMs (a holder that wrote no pid yet, or a pid that
// was reused). A takeover holds the lock's takeover file, created exclusively,
// and looks at the lock again under it: of two waiters that found the same
// stale lock, the second finds the lock the first has just taken, and leaves
// it. A holder gives up only its own lock.
const readText = (file) => { try { return fs.readFileSync(file, 'utf8') } catch { return null } }
const staleLock = (lock, holdMs) => {
  let age
  try { age = Date.now() - fs.statSync(lock).mtimeMs } catch { return false }
  const pid = Number((readText(lock) || '').split(' ')[0])
  return (Number.isInteger(pid) && pid > 0 && !alive(pid)) || age > holdMs
}
// Whether it removed the stale lock; false while another waiter takes it over.
const takeOver = (lock, holdMs) => {
  const guard = `${lock}.takeover`
  try {
    fs.closeSync(fs.openSync(guard, 'wx'))
  } catch (e) {
    if (e.code !== 'EEXIST') throw e
    // A takeover lasts milliseconds: a guard this old was left by a call
    // that was killed during one.
    try { if (Date.now() - fs.statSync(guard).mtimeMs > 10000) fs.rmSync(guard, { force: true }) } catch {}
    return false
  }
  try {
    if (!staleLock(lock, holdMs)) return false
    fs.rmSync(lock, { force: true })
    return true
  } finally {
    fs.rmSync(guard, { force: true })
  }
}
const withLock = (file, fn, { holdMs = 10000, waitMs = 30000 } = {}) => {
  const lock = `${file}.lock`
  const mine = `${process.pid} ${process.hrtime.bigint()}`
  fs.mkdirSync(path.dirname(lock), { recursive: true })
  const until = Date.now() + waitMs
  for (;;) {
    try {
      fs.writeFileSync(lock, mine, { flag: 'wx' })
      break
    } catch (e) {
      if (e.code !== 'EEXIST') throw e
      if (staleLock(lock, holdMs) && takeOver(lock, holdMs)) continue
      if (Date.now() > until) throw new Error(`${lock} stayed taken for ${Math.round(waitMs / 1000)} s`)
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 20)
    }
  }
  try { return fn() } finally { if (readText(lock) === mine) fs.rmSync(lock, { force: true }) }
}
const lockPathOf = (root) => path.join(root, '.omni', 'run.lock')
const runDirOf = (root, runId) => path.join(root, '.omni', 'runs', runId)
const appliedPathOf = (root, runId) => path.join(runDirOf(root, runId), 'applied.json')
const clarityPathOf = (root) => path.join(root, '.omni', 'clarity.json')
// Applied ops by step and opId: an opId of one step never answers another.
const recordOp = (root, runId, step, opId, body) => withLock(appliedPathOf(root, runId), () => {
  const applied = readJson(appliedPathOf(root, runId)) || {}
  applied[step] = { ...applied[step], [opId]: body }
  writeJson(appliedPathOf(root, runId), applied)
})
const recordedOp = (root, runId, step, opId) => {
  const ops = (readJson(appliedPathOf(root, runId)) || {})[step] || {}
  return Object.hasOwn(ops, opId) ? ops[opId] : null
}
// run.json holds the run: its feature, root, thread, checks, the tip of
// deliver/<slug> at its start, and the stories it landed.
const runPathOf = (root, runId) => path.join(runDirOf(root, runId), 'run.json')
const updateRun = (root, runId, change) => withLock(runPathOf(root, runId), () => {
  const info = change(readJson(runPathOf(root, runId)) || {})
  writeJson(runPathOf(root, runId), info)
  return info
})
// One line per step that ran: appends of whole short lines from parallel calls
// do not mix.
const timingsPathOf = (root, runId) => path.join(runDirOf(root, runId), 'timings.jsonl')
const recordTiming = (root, runId, entry) => { try { fs.appendFileSync(timingsPathOf(root, runId), `${JSON.stringify(entry)}\n`) } catch {} }
const timingsOf = (root, runId) => {
  let text = ''
  try { text = fs.readFileSync(timingsPathOf(root, runId), 'utf8') } catch {}
  return text.split('\n').filter(Boolean).flatMap((line) => { try { return [JSON.parse(line)] } catch { return [] } })
}

// ---- the lock --------------------------------------------------------------
// The lock holds the pid of the Claude Code process that runs the couriers.
// On macOS the chain is claude, then the courier's shell (/bin/zsh), then this
// node; a shell that runs its last command with exec leaves claude as the
// parent. So it is the nearest ancestor that is not a shell.
// That walk is checked on macOS only. OMNI_DELIVER_SESSION_PID sets the pid
// instead, for tests and for a machine where the walk finds the wrong
// process. null when ps cannot tell: such a lock goes stale only by its
// heartbeat.
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
// The lock is read and written under withLock, so that two starts at once
// cannot both take it, and a heartbeat never lands on a lock another run has
// just taken.
// Every call of the run says it is alive. false when the run no longer holds
// the lock: another run took it over, or the run finished.
const holdLock = (root, runId) => withLock(lockPathOf(root), () => {
  const held = readJson(lockPathOf(root))
  if (!held || held.run !== runId) return false
  writeJson(lockPathOf(root), { ...held, heartbeat: new Date().toISOString() })
  return true
})
const dropLock = (root, runId) => withLock(lockPathOf(root), () => {
  const held = readJson(lockPathOf(root))
  if (held && held.run === runId) fs.rmSync(lockPathOf(root), { force: true })
})

// ---- jobs: commands that take longer than a courier's Bash call ------------
const SELF = fileURLToPath(import.meta.url)
const SUPERVISE = '__supervise'
// A job's limit, from limitMinutes rounded to whole milliseconds; 45 minutes,
// the default limit of a command, when it is not a number above 0.
const JOB_MS = 45 * 60000
const jobLimitMs = (minutes) => (typeof minutes === 'number' && minutes > 0 ? Math.max(delay(minutes * 60000), 1) : JOB_MS)
// A stopped job's group gets SIGKILL this long after its SIGTERM.
const KILL_GRACE_MS = 10000
const graceMs = () => delay(Number(process.env.OMNI_DELIVER_KILL_GRACE_MS) || KILL_GRACE_MS)
// What job-wait and job-kill give of a log: its last 2 KB, from the first
// whole character.
const TAIL_BYTES = 2048
const jobsDirOf = (root, runId) => path.join(runDirOf(root, runId), 'jobs')
const jobFiles = (dir, id) => ({ job: path.join(dir, `${id}.job.json`), log: path.join(dir, `${id}.log`), result: path.join(dir, `${id}.json`) })
// A job's id: its key and name, and a number that makes it new in the run. The
// log is created with it, exclusively, so two job-starts at once never share one.
const newJobId = (dir, label) => {
  fs.mkdirSync(dir, { recursive: true })
  const base = label.replace(/[^A-Za-z0-9._-]+/g, '-')
  for (let n = 1; ; n++) {
    try {
      fs.closeSync(fs.openSync(jobFiles(dir, `${base}-${n}`).log, 'wx'))
      return `${base}-${n}`
    } catch (e) {
      if (e.code !== 'EEXIST') throw e
    }
  }
}
const jobAt = (dir, id) => {
  const files = jobFiles(dir, id)
  const job = readJson(files.job)
  return job ? { ...job, files } : null
}
const jobsOf = (root, runId) => {
  const dir = jobsDirOf(root, runId)
  let names = []
  try { names = fs.readdirSync(dir) } catch {}
  return names.filter((n) => n.endsWith('.job.json')).map((n) => jobAt(dir, n.slice(0, -'.job.json'.length))).filter(Boolean)
}
const runsIn = (root) => { try { return fs.readdirSync(path.join(root, '.omni', 'runs')) } catch { return [] } }
// Whether the job's supervisor runs: true when its pid runs this file's
// __supervise on the job, false when it runs nothing or something else (a
// pid is reused once its process ended), null when ps cannot tell.
const supervising = (runId, job) => {
  if (!Number.isInteger(job.supervisor) || !alive(job.supervisor)) return false
  const args = tryRun('ps', ['-ww', '-o', 'args=', '-p', String(job.supervisor)])
  if (args === null) return null
  return args.includes(SUPERVISE) && args.includes(path.join(runId, 'jobs', `${job.id}.job.json`))
}
// Whether the leader of a job's group, pid = pgid, still runs the job's
// command: whether it is the process the supervisor started for it, which
// ps shows running since the job started. Its arguments cannot tell: a shell
// runs a simple command by exec, without the variables set in front of it
// and its redirections. A pid that was reused started later, and is never
// signalled.
const leading = (job) => {
  if (!Number.isInteger(job.pgid) || job.pgid <= 0 || !alive(job.pgid)) return false
  // ps's etime: [[dd-]hh:]mm:ss.
  const m = (tryRun('ps', ['-o', 'etime=', '-p', String(job.pgid)]) || '').trim().match(/^(?:(\d+)-)?(?:(\d+):)?(\d+):(\d+)$/)
  const started = Date.parse(job.started)
  if (!m || !Number.isFinite(started)) return false
  const age = ((Number(m[1] || 0) * 24 + Number(m[2] || 0)) * 60 + Number(m[3])) * 60 + Number(m[4])
  return Math.abs((Date.now() - started) / 1000 - age) <= 2
}
const groupGone = (pgid) => { try { process.kill(-pgid, 0); return false } catch (e) { return e.code === 'ESRCH' } }
// A job whose supervisor is gone without its result is lost: its supervisor
// was killed itself.
const lose = (job) => {
  const lost = { ok: false, code: null, signal: null, timedOut: false, durationMs: null, started: job.started || null, ended: null, error: `the supervisor of the job ${job.id} ended without its result` }
  writeJson(job.files.result, lost)
  return lost
}
// A job's result once it has ended. One whose supervisor is gone is stopped,
// if its command still runs, and lost.
const endOf = (runId, job, ask = false) => {
  const result = readJson(job.files.result)
  if (result) return result
  if (alive(job.supervisor) && !(ask && supervising(runId, job) === false)) return null
  const late = readJson(job.files.result)
  if (late) return late
  stopJobs([{ run: runId, job }])
  return readJson(job.files.result) || lose(job)
}
const tailOf = (file) => {
  let fd
  try { fd = fs.openSync(file, 'r') } catch { return '' }
  try {
    const size = fs.fstatSync(fd).size
    const n = Math.min(size, TAIL_BYTES)
    const bytes = Buffer.alloc(n)
    fs.readSync(fd, bytes, 0, n, size - n)
    // A cut inside a character drops its continuation bytes.
    let from = 0
    if (size > n) while (from < n && (bytes[from] & 0xc0) === 0x80) from++
    return bytes.subarray(from).toString('utf8')
  } finally {
    fs.closeSync(fd)
  }
}
// The jobs of these runs that have not ended, as {run, job}.
const runningJobs = (root, runIds) => runIds.flatMap((run) => jobsOf(root, run).filter((job) => !fs.existsSync(job.files.result)).map((job) => ({ run, job })))
// Stops jobs that have not ended: SIGTERM to each one's supervisor, which
// stops the job's group as at its limit. A job whose supervisor is gone gets
// what the supervisor would have done: SIGTERM to its group when its leader
// still runs the job's command, SIGKILL after the grace unless the group is
// gone, and the result that says it is lost. A wait, the grace and 2 s at
// most, until they have ended. Only a supervisor that ps shows to be one is
// signalled, and ps that cannot tell signals nothing. Gives those stopped.
const stopJobs = (list) => {
  const supervised = []
  const orphans = []
  for (const entry of list) {
    if (fs.existsSync(entry.job.files.result)) continue
    const supervisor = supervising(entry.run, entry.job)
    if (supervisor === true) {
      try { process.kill(entry.job.supervisor, 'SIGTERM'); supervised.push(entry) } catch {}
    } else if (supervisor === false) {
      if (leading(entry.job)) { try { process.kill(-entry.job.pgid, 'SIGTERM'); orphans.push(entry) } catch {} }
      if (!orphans.includes(entry)) lose(entry.job)
    }
  }
  const killAt = Date.now() + graceMs()
  const until = killAt + 2000
  let killed = false
  const running = () => supervised.some(({ job }) => !fs.existsSync(job.files.result) && alive(job.supervisor)) || orphans.some(({ job }) => !groupGone(job.pgid))
  while (running() && Date.now() < until) {
    if (!killed && Date.now() >= killAt) {
      killed = true
      for (const { job } of orphans) { try { process.kill(-job.pgid, 'SIGKILL') } catch {} }
    }
    nap(50)
  }
  for (const { job } of orphans) lose(job)
  return [...supervised, ...orphans]
}

// The supervisor of one job, detached from the job-start that spawned it.
// It runs the command in a group of its own and writes the result when the
// command ends. At the limit, or on SIGTERM, the group gets SIGTERM, and
// SIGKILL after the grace, armed whatever the leader does: a member that
// ignores SIGTERM outlives its leader. The result comes once the group is gone
// (ESRCH) or has had its SIGKILL. Messages name the job, never its command.
const supervise = async (jobFile) => {
  const job = readJson(jobFile)
  if (!job) return
  const files = jobFiles(path.dirname(jobFile), job.id)
  const note = (text) => { try { fs.appendFileSync(files.log, text) } catch {} }
  const started = new Date()
  let child = null
  let stopping = null
  let signalled = false
  let killed = false
  let killer = null
  const toGroup = (signal) => { try { process.kill(-child.pid, signal) } catch {} }
  // The first reason stays. A stop asked for before the command started
  // reaches it once it has.
  const stop = (why) => {
    if (!stopping) stopping = why
    if (signalled || !child || !child.pid) return
    signalled = true
    toGroup('SIGTERM')
    killer = setTimeout(() => {
      killed = true
      toGroup('SIGKILL')
    }, graceMs())
  }
  process.on('SIGTERM', () => stop('request'))
  // One supervisor to a job: one started by a repeated job-start finds the job
  // started, and leaves.
  const mine = withLock(jobFile, () => {
    const now = readJson(jobFile)
    if (!now || now.started) return false
    const out = fs.openSync(files.log, 'a')
    try {
      // Straight to the log's descriptor: nothing is decoded on the way.
      child = spawn(now.cmd, { cwd: now.cwd, shell: true, detached: true, stdio: ['ignore', out, out] })
    } finally {
      fs.closeSync(out)
    }
    writeJson(jobFile, { ...now, supervisor: process.pid, pgid: child.pid ?? null, started: started.toISOString() })
    return true
  })
  if (!mine) return
  const exit = new Promise((resolve) => {
    child.once('exit', (code, signal) => resolve({ code, signal }))
    child.once('error', (error) => resolve({ error }))
  })
  if (stopping) stop(stopping)
  const limit = setTimeout(() => stop('limit'), delay(job.limitMs))
  const { code = null, signal = null, error } = await exit
  clearTimeout(limit)
  const why = stopping
  if (why && child.pid) {
    const gone = () => { try { process.kill(-child.pid, 0); return false } catch (e) { return e.code === 'ESRCH' } }
    while (!killed && !gone()) await pause(50)
    clearTimeout(killer)
  }
  if (error) note(`The job ${job.id} could not start: ${error.message}\n`)
  else if (why === 'limit') note(`\nTimed out: the job ${job.id} ran past its limit of ${span(job.limitMs)} and was stopped.\n`)
  else if (why === 'request') note(`\nStopped: the job ${job.id} was stopped before it ended.\n`)
  const ended = new Date()
  writeJson(files.result, {
    ok: !why && !error && code === 0,
    code,
    signal,
    timedOut: why === 'limit',
    durationMs: ended - started,
    started: started.toISOString(),
    ended: ended.toISOString(),
    ...(error ? { error: `the job ${job.id} could not start: ${error.message}` } : {}),
  })
  process.exit(0)
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
  // A post's body goes through a file in the run's folder: fed through stdin,
  // the synchronous spawn once left `omni post` waiting on its stdin for over
  // an hour, and the run with it.
  const post = (id, summary, body) => {
    const file = path.join(runDirOf(root, runId), `post-${id}-${process.pid}.md`)
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, body)
    try {
      omni(['post', id, '--summary', summary, '--body-file', file])
    } finally {
      try { fs.unlinkSync(file) } catch {}
    }
  }
  // A question for the human is a thread on the feature named after the story,
  // linked from the story, and the story is blocked until it concludes. An
  // edit sets all of a story's links, so one story's link writes go one at a
  // time, under a lock of the story: two thread-opens at once each linked the
  // story from what they read, and the second dropped the first's thread. A
  // link write is two calls, a minute each at most, so the lock is held for
  // minutes where the others are held for milliseconds.
  const linkAndBlock = (key, threadIds) => {
    withLock(path.join(root, '.omni', 'stories', key), () => {
      const fresh = readStory(key)
      const links = [...(fresh.links || []).map((l) => (l.thread_id ? `thread:${l.thread_id}:${l.label}` : `${l.doc_id}:${l.node_id}:${l.label}`)), ...threadIds.map((id) => `thread:${id}:question`)]
      omni(['story', 'edit', key, ...links.flatMap((l) => ['--link', l])])
    }, { holdMs: 10 * 60000, waitMs: 5 * 60000 })
    return write('block', key)
  }
  // A doc as `omni doc read` prints it, read once per call.
  const docs = new Map()
  const docText = (id) => {
    if (!docs.has(id)) docs.set(id, tryRun('omni', ['doc', 'read', id], { cwd: root, env: identity }) ?? `(the doc ${id} could not be read)`)
    return docs.get(id)
  }
  return { root, runId, feature, git, storiesDir, lane: path.join(storiesDir, '_lane'), branch, omni, listStories, readStory, write, post, linkAndBlock, docText, worktreeOf: (key) => path.join(storiesDir, key) }
}

const summary = (s) => ({ key: s.key, title: s.title, status: s.status, priority: s.priority ?? 3, blocked: !!s.blocked, holder: s.holder ? s.holder.run : null })
const statusOf = (s) => ({ key: s.key, status: s.status, blocked: !!s.blocked, holder: s.holder ? s.holder.run : null })
const pickableKeys = (b) => b.omni(['story', 'list', '--pickable']).split('\n').map((l) => l.trim().split(/\s+/)[0]).filter(Boolean)
const criterionOf = (c) => (typeof c === 'string' ? { text: c, human_check: false } : { text: c.text, human_check: !!c.human_check })
const isWorktree = (root, dir) => (tryRun('git', ['-C', root, 'worktree', 'list', '--porcelain']) || '').split('\n')
  .filter((l) => l.startsWith('worktree ')).some((l) => real(l.slice(9)) === real(dir))
// Undoes every edit in a worktree and removes what is untracked. Never -x:
// ignored build caches stay warm. Checked right before: git must work in the
// folder itself, not in a repository around it.
const cleanTree = (b, dir) => {
  if (!ownTree(dir)) throw new Error(`${dir} is not a worktree of its own: git there works in a repository around it, so it is not cleaned`)
  b.git(['-C', dir, 'reset', '-q', '--hard'])
  b.git(['-C', dir, 'clean', '-fdq'])
}
const isDir = (p) => { try { return fs.statSync(p).isDirectory() } catch { return false } }
const isAncestorIn = (root, a, c) => tryRun('git', ['-C', root, 'merge-base', '--is-ancestor', a, c]) !== null
// The ref a worktree has checked out, '' when its HEAD is detached.
const headRef = (dir) => (tryRun('git', ['-C', dir, 'symbolic-ref', '-q', 'HEAD']) || '').trim()
const hasBranch = (root, name) => tryRun('git', ['-C', root, 'rev-parse', '--verify', '--quiet', `refs/heads/${name}`]) !== null
// The paths git status shows in a worktree: changed tracked files, and
// untracked ones it does not ignore. -z gives each path as it is.
const changedFiles = (dir) => {
  const entries = run('git', ['-C', dir, 'status', '--porcelain', '-z']).split('\0')
  const files = []
  for (let i = 0; i < entries.length; i++) {
    if (!entries[i]) continue
    files.push(entries[i].slice(3))
    // A rename or copy is followed by the path it came from.
    if ('RC'.includes(entries[i][0])) i++
  }
  return files
}
// A list in a body: its first 20 entries.
const LISTED = 20
// The branch a rebase in progress in a worktree rebases ('' when it does not
// say), or null when no rebase is in progress there.
const rebasingIn = (dir) => {
  for (const name of ['rebase-merge', 'rebase-apply']) {
    const at = (tryRun('git', ['-C', dir, 'rev-parse', '--git-path', name]) || '').trim()
    if (at && isDir(path.resolve(dir, at))) return (readText(path.join(path.resolve(dir, at), 'head-name')) || '').trim()
  }
  return null
}

// ---- the lane --------------------------------------------------------------
// One story at a time is tested in the lane worktree, detached, and lands on
// deliver/<slug> by a compare-and-swap of the branch.
// A rebase replays each of the story's commits with the checkout's hooks, so
// it gets longer than a call. One that runs past this is the
// infrastructure's trouble, not a conflict.
const REBASE_MS = 5 * 60000
const rebaseMs = () => delay(Number(process.env.OMNI_DELIVER_REBASE_MS) || REBASE_MS)
const tipOf = (b) => b.git(['rev-parse', `refs/heads/${b.branch}`]).trim()
// The lane worktree, clean and detached at rev; made when it is missing.
const laneAt = (b, rev) => {
  if (!fs.existsSync(b.lane)) {
    tryRun('git', ['-C', b.root, 'worktree', 'prune'])
    fs.mkdirSync(b.storiesDir, { recursive: true })
    b.git(['worktree', 'add', '-q', '--detach', b.lane, rev])
    return
  }
  if (!isWorktree(b.root, b.lane)) throw new Error(`${b.lane} is not a worktree of ${b.root}, so it cannot be the lane worktree; move it away`)
  cleanTree(b, b.lane)
  b.git(['-C', b.lane, 'checkout', '-q', '--detach', rev])
}
const laneToTip = (b) => {
  const tip = tipOf(b)
  laneAt(b, tip)
  return { tip }
}
// A landed story's worktree leaves git at once, by a rename that takes no
// time, and its files go in the background: a build folder can take minutes
// to delete, past a courier's Bash call. A worktree that cannot be moved
// stays, and says so: the story has landed all the same. So does a folder in
// its place that is not the story's own worktree of this checkout on
// story/<key>, and the branch with it.
const removeWorktree = (b, key) => {
  const wt = b.worktreeOf(key)
  if (fs.existsSync(wt)) {
    if (!isWorktree(b.root, wt) || !ownTree(wt) || headRef(wt) !== `refs/heads/story/${key}`) return false
    try {
      const bin = path.join(b.storiesDir, '_trash')
      fs.mkdirSync(bin, { recursive: true })
      const trash = fs.mkdtempSync(path.join(bin, `${key}-`))
      fs.renameSync(wt, path.join(trash, key))
      const rm = spawn('rm', ['-rf', trash], { detached: true, stdio: 'ignore' })
      rm.on('error', () => {})
      rm.unref()
    } catch {
      return false
    }
  }
  tryRun('git', ['-C', b.root, 'worktree', 'prune'])
  tryRun('git', ['-C', b.root, 'branch', '-D', `story/${key}`])
  return !fs.existsSync(wt)
}
// A story's linked threads as `omni thread read` gives them, while the story
// is blocked on them; null when it is not. A thread that cannot be read
// throws, naming it.
const threadsOf = (b, story) => {
  if (!story.blocked || story.status === 'done' || story.status === 'cancelled') return null
  const ids = (story.links || []).map((l) => l.thread_id).filter(Boolean)
  if (ids.length === 0) return null
  return ids.map((id) => {
    try {
      return readThread(b, id)
    } catch (e) {
      throw new Error(`the thread ${id} could not be read: ${failure(e)}`)
    }
  })
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
  return { attempt: attemptOf(story), findings, uncounted, earlier: earlierOf(story) }
}
// The resolved findings of the attempts before the human's latest answers, in
// the order they came, one group per attempt: the next implementer still
// sees what failed before them. An attempt number that comes again after an
// answer starts a new group.
const earlierOf = (story) => {
  const groups = []
  for (const f of (story.findings || []).filter((x) => x.resolved)) {
    const m = String(f.text).match(ATTEMPT)
    if (!m) continue
    const item = { stage: f.stage, text: String(f.text).slice(m[0].length), files: f.files || [] }
    const last = groups[groups.length - 1]
    if (last && last.attempt === Number(m[1])) last.findings.push(item)
    else groups.push({ attempt: Number(m[1]), findings: [item] })
  }
  return groups
}
// A finding's text without the attempt numbers it starts with: an uncounted
// finding must never count as an attempt, and a counted one gets only its own.
const unnumbered = (text) => String(text).replace(/^(?:\[attempt \d+\] ?)+/, '')

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

// A step that writes gets, besides the board and the payload, its op's
// pending marker as `marker` when an earlier try of the same op was cut off or
// failed. It then checks the board for what that try did, and does only the
// rest. `mark` adds to the marker what such a later try needs to know.
// A write that puts the story in a state: a repeat writes only when the story
// is not in that state yet.
const stateWrite = (verb, names, flags, done) => (b, p, { marker }) => missing(p, 'key', ...names) || (() => {
  const was = marker ? b.readStory(p.key) : null
  if (was && done(was, p, b)) return statusOf(was)
  return statusOf(b.write(verb, p.key, flags(p)))
})()
const readThread = (b, id) => {
  const parsed = JSON.parse(b.omni(['thread', 'read', id, '--json']))
  return parsed.thread || parsed
}

// ---- the run's report ------------------------------------------------------
const took = (ms) => {
  if (ms < 1000) return '<1s'
  const s = Math.round(ms / 1000)
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`
}
const STAGES = ['implementing', 'review', 'testing']
// How long each story took: from its first step to its last; in each stage,
// from a claim the board took to the story's last step before its next one,
// which takes in the agents between them; in its rebases, which lane-prepare
// times apart from the lane's checkout; and in its jobs, by name in order. A
// count after a time says how many there were.
const timingTable = (records, jobs) => {
  const at = (iso) => Date.parse(iso)
  const keyed = records.filter((r) => r.key).sort((x, y) => at(x.started) - at(y.started))
  const ended = jobs.filter((j) => j.key && j.result && Number.isFinite(j.result.durationMs))
  const keys = [...new Set([...keyed.map((r) => r.key), ...ended.map((j) => j.key)])]
  if (keys.length === 0) return '- none'
  const names = [...new Set(ended.map((j) => j.name))].sort()
  const columns = [...STAGES, 'rebase', ...names.map((n) => `job ${n}`)]
  const add = (times, column, ms) => {
    const [sum, n] = times.get(column) || [0, 0]
    times.set(column, [sum + ms, n + 1])
  }
  const rows = keys.map((key) => {
    const mine = keyed.filter((r) => r.key === key)
    const theirs = ended.filter((j) => j.key === key)
    const spans = [...mine.map((r) => [at(r.started), at(r.ended)]), ...theirs.map((j) => [at(j.result.started), at(j.result.ended)])]
    const times = new Map()
    let open = null
    const close = () => { if (open) add(times, open.stage, open.to - open.from) }
    for (const r of mine) {
      if (r.step === 'claim' && r.ok) {
        close()
        open = { stage: r.stage, from: at(r.started), to: at(r.ended) }
      } else if (open) open.to = Math.max(open.to, at(r.ended))
      if (r.step === 'rebase') add(times, 'rebase', r.ms)
    }
    close()
    for (const j of theirs) add(times, `job ${j.name}`, j.result.durationMs)
    const cells = columns.map((c) => {
      const [sum, n] = times.get(c) || [0, 0]
      return n ? `${took(sum)}${n > 1 ? ` (${n})` : ''}` : '-'
    })
    return `| ${key} | ${took(Math.max(...spans.map((s) => s[1])) - Math.min(...spans.map((s) => s[0])))} | ${cells.join(' | ')} |`
  })
  const head = ['Story', 'Total', ...STAGES, 'rebase', ...names]
  return [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows, '', 'A time with (n) after it adds up n of them: claims of the stage, rebases, or runs of the job.'].join('\n')
}
// The report, modelled on the one omni-deliver.js posts: what the script knows
// (the outcomes, the council's counts, the blocked stories and their threads,
// the journeys) and what the helper does (the commits landed, the human checks
// left, the timings).
const reportText = (b, info, report) => {
  const list = (v) => (Array.isArray(v) ? v.filter((x) => x !== null && x !== undefined) : [])
  const lines = (items) => (items.length ? items.map((i) => `- ${i}`).join('\n') : '- none')
  let stories = []
  try { stories = b.listStories() } catch {}
  const titles = new Map(stories.map((s) => [s.key, s.title]))
  const named = (key) => (titles.get(key) ? `${key} ${titles.get(key)}` : String(key))
  const landed = list(info.landed)
  const commitsOf = (l) => (tryRun('git', ['-C', b.root, 'log', '--reverse', '--format=%h %s', `${l.base}..${l.final}`]) || '').split('\n').filter(Boolean)
  const council = report.council && typeof report.council === 'object' ? report.council : {}
  const count = (n) => (Number.isInteger(n) && n > 0 ? n : 0)
  const settled = count(council.settled)
  const checks = stories.flatMap((s) => (s.criteria || []).filter((c) => c && c.pending).map((c) => `${s.key}: ${criterionOf(c).text}`))
  const jobs = jobsOf(b.root, b.runId).map((job) => ({ ...job, result: readJson(job.files.result) }))
  const began = Date.parse(info.started)
  return `${[
    `# Run ${b.runId} on ${b.feature}`,
    ...list(report.notes).map(String),
    `## Landed on ${b.branch}\n${lines(landed.map((l) => `${named(l.key)}: ${commitsOf(l).join('; ') || 'no commits'}`))}${landed.length ? `\n\nBring them into your branch with \`git merge --ff-only ${b.branch}\`.` : ''}`,
    `## Blocked\n${lines(list(report.blocked).filter((s) => s.key).map((s) => `${named(s.key)}: thread ${list(s.threads).join(', ') || '(none linked)'}`))}`,
    `## Outcomes\n${lines(list(report.outcomes).filter((o) => o.key).map((o) => `${named(o.key)}: ${o.outcome}${o.detail ? `: ${o.detail}` : ''}`))}`,
    `## Council\n- ${settled} question${settled === 1 ? '' : 's'} settled by the council, ${count(council.asked)} sent to the human`,
    `## Pending human checks\n${lines(checks)}`,
    ...(Array.isArray(report.journeys) ? [`## Journeys\n${lines(list(report.journeys).map(String))}`] : []),
    `## Timings\n${Number.isFinite(began) ? `The run took ${took(Date.now() - began)}.\n\n` : ''}${timingTable(timingsOf(b.root, b.runId), jobs)}`,
  ].join('\n\n')}\n`
}

// ---- the steps of a run ----------------------------------------------------
const STEPS = {
  // The pickable stories that still have a worktree from an earlier attempt
  // come first, so that work begun is finished first; the board gives the
  // order besides: priority, then how many open stories each one unblocks,
  // then number.
  list: (b) => {
    const stories = b.listStories().map(summary)
    const keys = pickableKeys(b)
    const trees = new Set((tryRun('git', ['-C', b.root, 'worktree', 'list', '--porcelain']) || '').split('\n').filter((l) => l.startsWith('worktree ')).map((l) => real(l.slice(9))))
    const kept = keys.filter((key) => trees.has(real(b.worktreeOf(key))))
    return { stories, pickable: [...kept, ...keys.filter((key) => !kept.includes(key))] }
  },

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

  claim: stateWrite('claim', ['stage'], (p) => ['--stage', p.stage], (s, p, b) => s.status === p.stage && !!s.holder && s.holder.run === b.runId),
  release: stateWrite('release', [], () => [], (s, p, b) => !s.holder || s.holder.run !== b.runId),
  move: stateWrite('move', ['to'], (p) => ['--to', p.to], (s, p) => s.status === p.to),
  block: stateWrite('block', [], () => [], (s) => !!s.blocked),
  unblock: stateWrite('unblock', [], () => [], (s) => !s.blocked),

  // The findings of a failed attempt get its number, one past the attempts the
  // story has used; a failure that uses no attempt (counts false: a rebase
  // conflict, the infrastructure) gets none. An attempt that counts leaves at
  // least one finding, else the budget would not see it. The first try fixes
  // the number, so that a repeat finds what that try wrote and writes only
  // the rest, instead of counting the attempt twice.
  finding: (b, p, { marker, mark }) => missing(p, 'key', 'stage') || (() => {
    const counts = p.counts !== false
    if (!Array.isArray(p.findings) || p.findings.some((f) => !f || typeof f.text !== 'string') || (counts && p.findings.length === 0)) return { ok: false, error: 'the payload needs findings: [{text, files}], at least one when the attempt counts' }
    const story = b.readStory(p.key)
    const fixed = marker && Number.isInteger(marker.attempt)
    const attempt = fixed ? marker.attempt : attemptOf(story) + (counts ? 1 : 0)
    if (!fixed) mark({ attempt })
    const prefix = counts ? `[attempt ${attempt}] ` : ''
    const written = marker ? (story.findings || []).filter((f) => !f.resolved && f.stage === p.stage).map((f) => f.text) : []
    for (const f of p.findings) {
      const text = `${prefix}${unnumbered(f.text)}`
      const at = written.indexOf(text)
      if (at !== -1) {
        written.splice(at, 1)
        continue
      }
      b.write('finding', p.key, ['--stage', p.stage, '--text', text, ...(f.files || []).flatMap((file) => ['--file', file])])
    }
    return { key: p.key, attempt }
  })(),

  decision: (b, p, { marker }) => missing(p, 'key', 'question', 'choice', 'by') || (() => {
    const was = marker ? b.readStory(p.key) : null
    if (!(was && (was.decisions || []).some((d) => d.question === p.question && d.choice === p.choice && d.by === p.by))) b.write('decision', p.key, ['--question', p.question, '--choice', p.choice, '--by', p.by])
    return { key: p.key }
  })(),

  // What the story's branch changed since it left deliver/<slug> becomes its
  // files_changed, by which the board keeps stories on the same files apart.
  // -z gives each path as it is, where git otherwise quotes one that is not
  // ASCII.
  files: (b, p) => missing(p, 'key') || (() => {
    const wt = b.worktreeOf(p.key)
    if (!fs.existsSync(wt)) return { ok: false, error: `no worktree ${wt}` }
    const paths = b.git(['-C', wt, '-c', 'core.quotePath=false', 'diff', '--name-only', '-z', `refs/heads/${b.branch}...HEAD`]).split('\0').filter(Boolean)
    if (paths.length) b.write('files', p.key, paths)
    return { key: p.key, files: paths.length }
  })(),

  // A repeat uses the thread the first try opened, and posts, links and
  // blocks only what that try did not.
  'thread-open': (b, p, { marker, mark }) => missing(p, 'title', 'body') || (() => {
    let thread = marker && marker.thread
    const posted = thread ? (readThread(b, thread).posts || []).length > 0 : false
    if (!thread) {
      thread = b.omni(['thread', 'create', p.title]).trim()
      mark({ thread })
    }
    if (!posted) b.post(thread, p.title.slice(0, 200), p.body)
    if (!p.key) return { thread }
    try {
      const story = marker ? b.readStory(p.key) : null
      if (!story || !(story.links || []).some((l) => l.thread_id === thread)) b.linkAndBlock(p.key, [thread])
      else if (!story.blocked) b.write('block', p.key)
    } catch (e) {
      return { ok: false, thread, key: p.key, error: failure(e), retry: retryable(e) }
    }
    return { thread, key: p.key }
  })(),

  // A blocked story whose linked threads all have a conclusion gets each
  // conclusion as a human decision and is unblocked: a thread's conclusion is
  // the human's answer. The answer restarts the story's attempt budget: its
  // open findings are resolved, and attempts count from there (attemptOf).
  // A thread that cannot be read keeps its story blocked, says so in the log,
  // and the pass goes on. Any other failure stops the pass with the keys it
  // unblocked so far; the stories after it are tried again by the next pass.
  // A repeat runs the pass again: what the first try wrote is not written twice.
  'unblock-pass': (b) => {
    const unblocked = []
    const log = []
    try {
      for (const story of b.listStories()) {
        let threads
        try {
          threads = threadsOf(b, story)
        } catch (e) {
          log.push(`${story.key} stays blocked: ${e.message}`)
          continue
        }
        if (!threads || threads.some((t) => !t.conclusion)) continue
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
      return { ok: false, unblocked, log, error: failure(e), retry: retryable(e) }
    }
    return { unblocked, log }
  },

  // A new worktree starts at the tip of deliver/<slug>, what the lane last
  // landed, on a new branch story/<key>. A branch story/<key> without its
  // worktree keeps its work: the worktree is made again on it. An existing
  // worktree is kept, with what earlier attempts committed and left in it,
  // when it is a worktree of the checkout on story/<key>; any other folder in
  // its place is an error. The marker says what was there before the first
  // try, so that a repeat answers created as the first try would have. A
  // worktree the first try was still making counts only when it is clean and
  // on story/<key>, since a checkout that was killed leaves one with most of
  // its files missing; else it is removed and made again.
  worktree: (b, p, { marker, mark }) => missing(p, 'key') || (() => {
    const wt = b.worktreeOf(p.key)
    const branch = `story/${p.key}`
    const known = marker && typeof marker.existed === 'boolean'
    const { existed, fresh } = known ? marker : { existed: fs.existsSync(wt), fresh: !hasBranch(b.root, branch) }
    if (!known) mark({ existed, fresh })
    const onBranch = () => isWorktree(b.root, wt) && ownTree(wt) && headRef(wt) === `refs/heads/${branch}`
    const clean = () => { try { return changedFiles(wt).length === 0 } catch { return false } }
    if (fs.existsSync(wt) && !existed && isWorktree(b.root, wt) && !(onBranch() && clean())) {
      // -f twice: git keeps a worktree it was making locked.
      tryRun('git', ['-C', b.root, 'worktree', 'remove', '--force', '--force', wt])
      if (fs.existsSync(wt)) return { ok: false, error: `${wt}, a worktree the first try was making, could not be removed; move it away` }
    }
    if (fs.existsSync(wt)) {
      if (!onBranch()) return { ok: false, error: `${wt} is not a worktree of ${b.root} on ${branch}; move it away` }
      return { key: p.key, path: wt, branch, created: fresh }
    }
    tryRun('git', ['-C', b.root, 'worktree', 'prune'])
    fs.mkdirSync(b.storiesDir, { recursive: true })
    if (hasBranch(b.root, branch)) b.git(['worktree', 'add', wt, branch])
    else b.git(['worktree', 'add', wt, '-b', branch, tipOf(b)])
    return { key: p.key, path: wt, branch, created: fresh }
  })(),

  // The wait between retries: the script has no clock.
  sleep: async (b, p) => {
    const seconds = Math.min(Math.max(Number(p.seconds) || 0, 0), MAX_SLEEP_S)
    await pause(seconds * 1000)
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

  // The run's checks live in its run.json: start writes those omni checks
  // gives, and this merges in others. omni checks gives fast, stack_up and
  // stack, so this is how a run gets seed or lint, or a check its project
  // lacks.
  'checks-set': (b, p) => {
    if (!p.checks || typeof p.checks !== 'object' || Array.isArray(p.checks)) return { ok: false, error: 'the payload needs checks: {name: command}' }
    const unknown = Object.keys(p.checks).filter((name) => !CHECKS.includes(name))
    if (unknown.length) return { ok: false, error: `no check ${unknown.join(', ')}; the checks are ${CHECKS.join(', ')}` }
    const odd = Object.keys(p.checks).filter((name) => typeof p.checks[name] !== 'string')
    if (odd.length) return { ok: false, error: `a check is a command, as a string: ${odd.join(', ')}` }
    return { checks: updateRun(b.root, b.runId, (info) => ({ ...info, checks: { ...info.checks, ...p.checks } })).checks }
  },

  // Starts a job and returns once its supervisor has started the command. A
  // repeat after a try that was cut off goes on with the job that try made.
  'job-start': (b, p, { marker, mark }) => missing(p, 'name', 'cmd', 'cwd') || (async () => {
    if (p.key !== undefined && (typeof p.key !== 'string' || !p.key)) return { ok: false, error: 'the payload key, when given, is a story key' }
    const cwd = path.resolve(b.root, p.cwd)
    if (!isDir(cwd)) return { ok: false, error: `the job ${p.name} needs a folder to run in; there is none at ${cwd}` }
    const dir = jobsDirOf(b.root, b.runId)
    let id = marker && marker.id
    if (!id || !fs.existsSync(jobFiles(dir, id).job)) {
      id = newJobId(dir, p.key ? `${p.key}-${p.name}` : p.name)
      writeJson(jobFiles(dir, id).job, { id, name: p.name, key: p.key || null, cmd: p.cmd, cwd, limitMs: jobLimitMs(p.limitMinutes) })
      mark({ id })
    }
    const files = jobFiles(dir, id)
    const started = () => !!(readJson(files.job) || {}).started
    if (!started()) {
      const supervisor = spawn(process.execPath, [SELF, SUPERVISE, files.job], { cwd: b.root, detached: true, stdio: 'ignore' })
      let gone = false
      supervisor.on('exit', () => { gone = true })
      supervisor.on('error', () => { gone = true })
      supervisor.unref()
      for (const until = Date.now() + 10000; !started(); await pause(20)) {
        if (gone || Date.now() > until) return { ok: false, id, error: `the job ${id} did not start: its supervisor ${gone ? 'ended first' : 'did not start it within 10 s'}`, retry: true }
      }
    }
    return { id, log: files.log }
  })(),

  // The job's result and the tail of its log once it has ended; done false
  // after maxSeconds (480 at most) while it runs.
  'job-wait': (b, p) => missing(p, 'id') || (async () => {
    const job = /^[\w.-]+$/.test(p.id) ? jobAt(jobsDirOf(b.root, b.runId), p.id) : null
    if (!job) return { ok: false, error: `no job ${p.id} in run ${b.runId}` }
    const until = Date.now() + waitSeconds(p.maxSeconds) * 1000
    for (;;) {
      // At the end of the wait ps also says whether the pid is still the supervisor's.
      const result = endOf(b.runId, job, Date.now() >= until)
      if (result) return { id: p.id, done: true, result, tail: tailOf(job.files.log) }
      if (Date.now() >= until) return { id: p.id, done: false }
      await pause(Math.min(100, until - Date.now()))
    }
  })(),

  'job-kill': (b, p) => missing(p, 'id') || (() => {
    const job = /^[\w.-]+$/.test(p.id) ? jobAt(jobsDirOf(b.root, b.runId), p.id) : null
    if (!job) return { ok: false, error: `no job ${p.id} in run ${b.runId}` }
    const killed = !fs.existsSync(job.files.result) && stopJobs([{ run: b.runId, job }]).length > 0
    const result = readJson(job.files.result)
    return { id: p.id, killed, done: !!result, ...(result ? { result, tail: tailOf(job.files.log) } : {}) }
  })(),

  // Rebases story/<key> onto the tip of deliver/<slug> in the story's
  // worktree, then checks the story's tip out, detached, in a clean lane
  // worktree. Git runs only in the story's own worktree of this checkout, on
  // story/<key>: in a folder that lost its .git file, git works in a
  // repository around it, whose rebase an abort would undo. Outcomes, not
  // errors, first those found before the rebase: a story without its
  // worktree or branch, or a folder that is not its worktree (missing); a
  // rebase in progress there (infra), but for this op's own, which a try
  // that was cut off left half done and the repeat undoes; another branch or
  // a detached HEAD checked out (wrong_branch), whose commits would not land;
  // uncommitted changes (dirty), which would not land either. Then the
  // rebase: unmerged paths are a conflict, after which the rebase is undone
  // and their files come back; a rebase past its limit, or one that failed
  // any other way, on a lock or a hook, is infra, with git's message.
  'lane-prepare': (b, p, { marker }) => missing(p, 'key') || (() => {
    const wt = b.worktreeOf(p.key)
    const branch = `story/${p.key}`
    const ref = `refs/heads/${branch}`
    const outcome = (name, fields) => ({ key: p.key, outcome: name, ...fields })
    if (!fs.existsSync(wt)) return outcome('missing', { reason: `no worktree ${wt}` })
    if (!hasBranch(b.root, branch)) return outcome('missing', { reason: `no branch ${branch}` })
    if (!isWorktree(b.root, wt) || !ownTree(wt)) return outcome('missing', { reason: `${wt} is not a worktree of ${b.root} of its own, so git does nothing in it` })
    if (marker && rebasingIn(wt) === ref) tryRun('git', ['-C', wt, 'rebase', '--abort'])
    const rebasing = rebasingIn(wt)
    if (rebasing !== null) return outcome('infra', { reason: `a rebase${rebasing ? ` of ${rebasing.replace(/^refs\/heads\//, '')}` : ''} is in progress in ${wt}` })
    const head = headRef(wt)
    if (head !== ref) {
      const commits = (tryRun('git', ['-C', wt, 'log', '--format=%h %s', `${ref}..HEAD`]) || '').split('\n').filter(Boolean)
      const name = head ? head.replace(/^refs\/heads\//, '') : null
      const what = name ? `the branch ${name} checked out` : `a detached HEAD at ${short(b.git(['-C', wt, 'rev-parse', 'HEAD']).trim())}`
      const lost = commits.length ? `, so its ${commits.length} commit${commits.length === 1 ? '' : 's'} not on ${branch} would be lost (${commits.slice(0, 3).join('; ')}${commits.length > 3 ? '; and more' : ''})` : ''
      return outcome('wrong_branch', { head: name, commits: commits.slice(0, LISTED), reason: `${wt} has ${what}, not ${branch}: only ${branch} lands${lost}. Commit on ${branch}` })
    }
    const dirty = changedFiles(wt)
    if (dirty.length) return outcome('dirty', { files: dirty.slice(0, LISTED), count: dirty.length, reason: `${wt} has uncommitted changes in ${dirty.length} file${dirty.length === 1 ? '' : 's'}, which would not land: commit or discard them` })
    const base = tipOf(b)
    // The rebase is timed apart from the step, whose lane checkout can take
    // longer than the rebase itself.
    const began = new Date()
    let failed = null
    try {
      b.git(['-C', wt, 'rebase', '-q', base], { timeout: rebaseMs() })
    } catch (e) {
      failed = e
    }
    const ended = new Date()
    recordTiming(b.root, b.runId, { step: 'rebase', op: p.opId, key: p.key, started: began.toISOString(), ended: ended.toISOString(), ms: ended - began, ok: !failed })
    if (failed) {
      const files = failed.timedOut ? [] : (tryRun('git', ['-C', wt, '-c', 'core.quotePath=false', 'diff', '--name-only', '-z', '--diff-filter=U']) || '').split('\0').filter(Boolean)
      if (rebasingIn(wt) !== null) tryRun('git', ['-C', wt, 'rebase', '--abort'])
      if (failed.timedOut) return outcome('infra', { reason: `the rebase of ${branch} onto ${b.branch} ran past its limit of ${span(rebaseMs())}` })
      if (files.length) return outcome('conflict', { files })
      return outcome('infra', { reason: `the rebase of ${branch} onto ${b.branch} failed: ${failure(failed)}` })
    }
    const candidate = b.git(['rev-parse', ref]).trim()
    laneAt(b, candidate)
    return outcome('ready', { base, candidate })
  })(),

  // Throws away what is uncommitted in the lane worktree, as start does
  // (reset --hard and clean -fd, never -x: ignored build caches stay warm),
  // and says what: after the test author and before its check, so that the
  // check runs on exactly what lands.
  'lane-clean': (b) => {
    if (!isWorktree(b.root, b.lane) || !ownTree(b.lane)) return { ok: false, error: `${b.lane} is not a worktree of ${b.root}, so it cannot be the lane worktree; move it away` }
    const files = changedFiles(b.lane)
    cleanTree(b, b.lane)
    return { discarded: files.slice(0, LISTED), count: files.length }
  },

  // The lane worktree at the tip of deliver/<slug>, clean: lane-base for the
  // rerun of a failed check on the base, lane-reset after a failure.
  'lane-base': (b) => laneToTip(b),
  'lane-reset': (b) => laneToTip(b),

  // Lands the lane worktree's HEAD, the tested candidate with what the test
  // author committed on top, by a compare-and-swap of deliver/<slug> from the
  // base. Refused (landed false) unless that HEAD is detached, with nothing
  // uncommitted, descends from the base and holds story/<key>, and unless the
  // branch is still at the base. A branch at the lane's HEAD proves nothing:
  // a commit made in the lane with deliver/<slug> checked out moves the
  // branch there too. So a lane with a branch checked out puts deliver/<slug>
  // back at the base, by a compare-and-swap from where it is, before it
  // detaches the lane: a repeat cut off between the two finds the branch
  // checked out still, and the branch already back. Only this op's record of
  // the swap it made lets a repeat, after a try that was cut off, skip it.
  // Then the landed tree must be the tested one, and the story is done: its
  // files recorded, its criteria ticked (a human check pending), its open
  // findings resolved, its worktree and branch gone.
  'lane-land': (b, p, { marker, mark }) => missing(p, 'key', 'base') || (() => {
    const refused = (reason) => ({ key: p.key, landed: false, reason })
    let final = marker && marker.swapped
    if (!final) {
      const head = headRef(b.lane)
      if (head) {
        const tip = tipOf(b)
        if (tip !== p.base) b.git(['update-ref', `refs/heads/${b.branch}`, p.base, tip])
        b.git(['-C', b.lane, 'checkout', '-q', '--detach'])
        return refused(`the lane worktree had ${head.replace(/^refs\/heads\//, '')} checked out, not a detached HEAD: a commit there moves that branch without the compare-and-swap. ${tip !== p.base ? `${b.branch} is back at the base ${short(p.base)}, from ${short(tip)}, and the` : 'The'} lane is now detached`)
      }
      const dirty = changedFiles(b.lane)
      if (dirty.length) return refused(`the lane worktree has uncommitted changes in ${dirty.length} file${dirty.length === 1 ? '' : 's'} (${dirty.slice(0, 5).join(', ')}${dirty.length > 5 ? ', and more' : ''}): what lands must be what was checked, so lane-clean it and check it again`)
      final = b.git(['-C', b.lane, 'rev-parse', 'HEAD']).trim()
      if (!isAncestorIn(b.root, p.base, final)) return refused(`the lane's HEAD ${short(final)} does not descend from the base ${short(p.base)}`)
      const story = (tryRun('git', ['-C', b.root, 'rev-parse', '--verify', '--quiet', `refs/heads/story/${p.key}`]) || '').trim()
      if (!story) return refused(`there is no branch story/${p.key} to land`)
      if (!isAncestorIn(b.root, story, final)) return refused(`the lane's HEAD ${short(final)} does not hold story/${p.key} at ${short(story)}`)
      try {
        b.git(['update-ref', `refs/heads/${b.branch}`, final, p.base])
      } catch (e) {
        const tip = tipOf(b)
        if (tip !== p.base) return refused(`${b.branch} moved to ${short(tip)} since the base ${short(p.base)}`)
        throw e
      }
      mark({ swapped: final })
    }
    const landedTree = b.git(['rev-parse', `refs/heads/${b.branch}^{tree}`]).trim()
    const finalTree = b.git(['rev-parse', `${final}^{tree}`]).trim()
    if (landedTree !== finalTree) return { ok: false, key: p.key, error: `${b.branch} holds the tree ${short(landedTree)} after the landing, not ${short(finalTree)}, the tree of ${short(final)} that was tested` }
    updateRun(b.root, b.runId, (info) => ({ ...info, landed: [...(info.landed || []).filter((l) => l.final !== final), { key: p.key, base: p.base, final }] }))
    const files = b.git(['-c', 'core.quotePath=false', 'diff', '--name-only', '-z', p.base, final]).split('\0').filter(Boolean)
    try {
      if (files.length) b.write('files', p.key, files)
      const story = b.readStory(p.key)
      ;(story.criteria || []).forEach((c, i) => { if (!c.ticked) b.write('tick', p.key, ['--criterion', String(i), ...(criterionOf(c).human_check ? ['--pending'] : [])]) })
      ;(story.findings || []).forEach((f, i) => { if (!f.resolved) b.write('resolve', p.key, ['--finding', String(i)]) })
      if (story.status !== 'done') b.write('move', p.key, ['--to', 'done'])
    } catch (e) {
      return { ok: false, key: p.key, landed: true, base: p.base, final, error: `landed on ${b.branch} at ${short(final)}, but the story is not done: ${failure(e)}`, retry: retryable(e) }
    }
    return { key: p.key, landed: true, base: p.base, final, files: files.length, removed: removeWorktree(b, p.key) }
  })(),

  // Posts the run's report on its thread, from a body file, after stopping
  // the run's jobs that still run, and gives the lock up. The report is also
  // kept at .omni/runs/<run>/report.md, which is all there is of it when the
  // post fails: the run ends and gives its lock up all the same, as
  // omni-deliver.js does, and a repeat of this finish goes on without the
  // lock, to post it then. A repeat posts only when the first try did not:
  // its marker says so, or the Run thread holds the report already, as after
  // a try that was cut off right after its post.
  finish: (b, p, { info, marker, mark }) => {
    if (!p.report || typeof p.report !== 'object' || Array.isArray(p.report)) return { ok: false, error: 'the payload needs report: {outcomes, council, blocked}' }
    try {
      const stopped = stopJobs(runningJobs(b.root, [b.runId])).map(({ job }) => job.id)
      const text = reportText(b, info, p.report)
      const report = path.join(runDirOf(b.root, b.runId), 'report.md')
      fs.writeFileSync(report, text)
      const summary = `Report of ${b.runId}`
      if (!(marker && (marker.posted || (readThread(b, info.thread).posts || []).some((post) => post.summary === summary)))) {
        try {
          b.post(info.thread, summary, text)
        } catch (e) {
          return { ok: false, thread: info.thread || null, posted: false, report, stopped, error: `the report could not be posted: ${failure(e)}`, retry: retryable(e) }
        }
        mark({ posted: true })
      }
      return { thread: info.thread, posted: true, report, stopped }
    } finally {
      // Marked first, so that a repeat that finds no lock knows this finish
      // gave it up.
      mark({ released: true })
      dropLock(b.root, b.runId)
    }
  },
}
// The steps whose bodies are recorded under their opId, so that a repeat does
// not write twice. The others only read, or wait: a repeat runs them again.
const WRITES = new Set(['claim', 'release', 'move', 'block', 'unblock', 'finding', 'decision', 'files', 'thread-open', 'unblock-pass', 'worktree', 'clarity-record', 'checks-set', 'job-start', 'job-kill', 'lane-prepare', 'lane-clean', 'lane-base', 'lane-reset', 'lane-land', 'finish'])

// ---- start: the lock, reconcile, the deliver branch, the checks ------------
// A start repeated by a retried courier comes within minutes of the first.
// The same op from the same session later is a new run the human started.
const REPLAY_MS = 5 * 60000
const start = (flags) => {
  const { op: opId, feature } = flags
  const force = flags.force === true
  const received = fnv1a(JSON.stringify({ opId, feature, force }))
  // Flags that do not hash to --hash came wrong from the courier.
  if (!opId || !feature || !flags.hash) return { ok: false, received, error: 'start needs --op <opId>, --feature <name> and --hash <fnv1a of {opId, feature, force}>', retry: true }
  if (flags.hash !== received) return { ok: false, received, error: `the flags hash to ${received}, not ${flags.hash}: run the command again exactly`, retry: true }
  const top = tryRun('git', ['rev-parse', '--show-toplevel'])
  if (top === null) return { ok: false, received, error: `${process.cwd()} is not in a git checkout`, retry: false }
  const root = top.trim()
  const session = sessionPid()
  const lockPath = lockPathOf(root)
  const log = []
  const now = new Date()
  const runId = `run-${stamp(now)}-${process.pid}`
  // The lock is read and taken in one step: of two starts at once, one waits
  // for the other and then finds its lock.
  const taken = withLock(lockPath, () => {
    let raw = null
    try { raw = fs.readFileSync(lockPath, 'utf8') } catch {}
    const held = raw === null ? null : readJson(lockPath)
    if (raw !== null && !held) log.push('The lock file could not be read; taking it over')
    if (held) {
      // The same start again, from this session, soon after and before its
      // run made any call: its courier failed after the helper ran. Its
      // record answers while its run holds the lock, a forced start's too:
      // the retry of a start with force would take over the lock of the run
      // its first try started.
      const retried = session !== null && held.pid === session && held.op === opId && held.feature === feature && held.heartbeat === held.started && Date.now() - Date.parse(held.started) < REPLAY_MS
      const recorded = retried ? recordedOp(root, held.run, 'start', opId) : null
      if (recorded && recorded.received === received) return { replay: recorded }
      const stale = retried ? 'its start, from this session, ended before it finished' : staleness(held)
      if (!stale && !force) return { refused: `the tree is held by run ${held.run} for feature ${held.feature}, last heard from at ${held.heartbeat || held.started}; start with force if that run is dead` }
      log.push(`Took over the lock of run ${held.run} for feature ${held.feature}: ${stale || 'start with force'}`)
    }
    writeJson(lockPath, { run: runId, feature, pid: session, op: opId, started: now.toISOString(), heartbeat: now.toISOString() })
    return {}
  })
  if (taken.replay) return taken.replay
  if (taken.refused) return { ok: false, received, error: taken.refused, retry: false }
  let body
  try {
    body = startRun(boardOf(root, runId, feature), log)
  } catch (e) {
    body = { ok: false, error: failure(e), retry: retryable(e) }
  }
  // A start that stops gives the lock up, and says what it did before.
  if (body.ok === false) {
    dropLock(root, runId)
    return { ok: false, received, error: body.error, retry: body.retry === true, log }
  }
  body = { ok: true, received, ...body }
  fs.mkdirSync(path.join(runDirOf(root, runId), 'in'), { recursive: true })
  writeJson(runPathOf(root, runId), { run: runId, feature, root, thread: body.thread, started: now.toISOString(), tip: body.tip, checks: body.checks, landed: [] })
  recordOp(root, runId, 'start', opId, body)
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
    return { ok: false, error: `the checks could not be read: ${failure(e)}`, retry: retryable(e) }
  }
  // Pruned first: a lane folder that lost its .git file is still listed until
  // the prune, and would pass for the lane worktree. git there would work in
  // whatever repository is around the folder, and the clean below would hit
  // that one.
  tryRun('git', ['-C', root, 'worktree', 'prune'])
  if (fs.existsSync(b.lane) && (!isWorktree(root, b.lane) || !ownTree(b.lane))) return { ok: false, error: `${b.lane} is not a worktree of ${root}, so it cannot be the lane worktree; move it away` }
  // The stories folder lies beside the checkout, where a session's sandbox
  // lets nothing write unless the human added it: found here, not at the
  // first worktree.
  try {
    fs.mkdirSync(b.storiesDir, { recursive: true })
    const probe = path.join(b.storiesDir, `.omni-write-${process.pid}`)
    fs.writeFileSync(probe, '')
    fs.rmSync(probe, { force: true })
  } catch (e) {
    return { ok: false, error: `${b.storiesDir} cannot be written (${e.code || e.message}): add ${b.storiesDir} to this session's working directories, then start again` }
  }

  // The run's thread, named as omni-deliver.js names it: finish posts the
  // report there.
  const thread = b.omni(['thread', 'create', `Run ${b.runId}`]).trim()
  if (move === 'create') git(['update-ref', `refs/heads/${b.branch}`, head, ''])
  if (move === 'fast-forward') git(['update-ref', `refs/heads/${b.branch}`, head, was])
  const tip = move === 'create' || move === 'fast-forward' ? head : was
  log.push({
    create: `${b.branch} created at ${short(head)}, the checkout's HEAD`,
    same: `${b.branch} is at ${short(was)}, the checkout's HEAD`,
    'fast-forward': `${b.branch} fast-forwarded from ${short(was)} to ${short(head)}, the checkout's HEAD`,
    ahead: `${b.branch} kept at ${short(was)}, ahead of the checkout's HEAD ${short(head)}`,
  }[move])

  // The jobs of earlier runs that still run, such as a check in the lane
  // worktree: their supervisors are detached, so a workflow that stopped left
  // them going. This run holds the lock, so those runs are over.
  for (const { run, job } of stopJobs(runningJobs(root, runsIn(root)))) log.push(`Stopped the job ${job.id} of run ${run}, which had ended`)

  // The lane worktree stays from run to run, detached.
  if (fs.existsSync(b.lane)) {
    cleanTree(b, b.lane)
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
  // The feature's stories as keys and counts: the script lists them itself
  // when it needs more.
  const stories = b.listStories()
  const counts = {}
  for (const s of stories) counts[s.status] = (counts[s.status] || 0) + 1
  return {
    run: b.runId,
    root,
    branch: b.branch,
    tip,
    lane: b.lane,
    thread,
    checks: Object.fromEntries(CHECKS.map((name) => [name, String((checks && checks[name]) || '')])),
    stories: { keys: stories.map((s) => s.key), counts, blocked: stories.filter((s) => s.blocked).length },
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
  // What the courier ran or wrote came wrong: the same command, as the
  // script gave it, can work.
  if (!step) return { ok: false, received: null, error: `no step ${flags.step}; the steps are start, ${Object.keys(STEPS).join(', ')}`, retry: true }
  if (!flags.run || !flags.payload || !flags.hash) return { ok: false, received: null, error: `${flags.step} needs --run <runId> --payload <file> --hash <fnv1a of the payload>`, retry: true }
  const file = path.resolve(flags.payload)
  const where = file.match(PAYLOAD_PATH)
  if (!where || where[2] !== flags.run) return { ok: false, received: null, error: `the payload goes in <root>/.omni/runs/${flags.run}/in/<opId>.json, not ${file}`, retry: true }
  const root = where[1]
  let text
  try { text = fs.readFileSync(file, 'utf8') } catch { return { ok: false, received: null, error: `no payload at ${file}`, retry: true } }
  fs.rmSync(file, { force: true })
  if (text.endsWith('\n')) text = text.slice(0, -1)
  const received = fnv1a(text)
  // Before anything else: a payload the courier copied wrong is neither acted
  // on nor looked up.
  if (received !== flags.hash) return { ok: false, received, error: `the payload hashes to ${received}, not ${flags.hash}: write it again exactly`, retry: true }
  // The payload is the script's own from here on: what is refused now is
  // refused again.
  const refuse = (error) => ({ ok: false, received, error, retry: false })
  let payload
  try { payload = JSON.parse(text) } catch { return refuse('the payload is not JSON') }
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) return refuse('the payload is not a JSON object')
  const info = readJson(runPathOf(root, flags.run))
  if (!info) return refuse(`no run ${flags.run} in ${root}`)
  const writes = WRITES.has(flags.step)
  let marker = null
  if (writes) {
    if (typeof payload.opId !== 'string' || !payload.opId) return refuse(`${flags.step} writes, so its payload needs an opId`)
    const recorded = recordedOp(root, flags.run, flags.step, payload.opId)
    if (recorded && recorded.received !== received) return refuse(`the op ${payload.opId} was applied with another payload, which hashed to ${recorded.received}; a new payload needs a new opId`)
    if (recorded && !recorded.pending) return recorded
    marker = recorded
  }
  // finish gives the lock up even when its post failed: its repeat goes on
  // without the lock, unless another run has taken it since.
  const released = flags.step === 'finish' && marker && marker.released === true && !fs.existsSync(lockPathOf(root))
  if (!released && !holdLock(root, flags.run)) return refuse(`run ${flags.run} no longer holds the lock`)
  // The marker goes down before the first write, and the body replaces it
  // once the step worked. A step that failed leaves it: the script tries a
  // failed write again with the same opId, and the repeat carries it on.
  let pending = marker || { pending: true, received }
  const mark = (fields) => {
    if (!writes) return
    pending = { ...pending, ...fields }
    recordOp(root, flags.run, flags.step, payload.opId, pending)
  }
  if (writes && !marker) recordOp(root, flags.run, flags.step, payload.opId, pending)
  const began = new Date()
  let body
  try {
    body = { ok: true, received, ...(await step(boardOf(root, flags.run, info.feature), payload, { info, marker, mark })) }
  } catch (e) {
    body = { ok: false, received, error: failure(e), retry: retryable(e) }
  }
  // A step's own refusal, of a payload or of what it found, is final.
  if (body.ok === false && typeof body.retry !== 'boolean') body.retry = false
  const ended = new Date()
  recordTiming(root, flags.run, {
    step: flags.step,
    op: payload.opId,
    ...(typeof payload.key === 'string' ? { key: payload.key } : {}),
    ...(typeof payload.stage === 'string' ? { stage: payload.stage } : {}),
    started: began.toISOString(),
    ended: ended.toISOString(),
    ms: ended - began,
    ok: body.ok,
  })
  if (writes && body.ok) recordOp(root, flags.run, flags.step, payload.opId, body)
  return body
}

const flagsOf = (argv) => {
  const flags = { step: argv[0] }
  for (let i = 1; i < argv.length; i++) {
    if (argv[i] === '--force') flags.force = true
    else if (['--run', '--payload', '--op', '--feature', '--hash'].includes(argv[i])) flags[argv[i].slice(2)] = argv[++i]
    else flags.unknown = argv[i]
  }
  return flags
}

export async function main(argv = process.argv.slice(2)) {
  // A job's supervisor, which job-start spawns detached: it prints nothing,
  // and the job's files say what happened.
  if (argv[0] === SUPERVISE) return supervise(argv[1])
  const flags = flagsOf(argv)
  let body
  try {
    if (flags.unknown !== undefined) body = { ok: false, received: null, error: `unknown argument ${flags.unknown}; see the header of workflows/deliver-step.mjs`, retry: true }
    else body = flags.step === 'start' ? start(flags) : await runStep(flags)
  } catch (e) {
    body = { ok: false, received: null, error: failure(e), retry: retryable(e) }
  }
  process.stdout.write(`${JSON.stringify({ body, hash: fnv1a(JSON.stringify(body)) })}\n`)
}

// Run as a command, also through the link at ~/.claude/omni/deliver-step.mjs:
// process.argv[1] is then the link, and import.meta.url the file it points to.
const invoked = (() => { try { return fs.realpathSync(process.argv[1]) } catch { return null } })()
if (invoked === fileURLToPath(import.meta.url)) main()
