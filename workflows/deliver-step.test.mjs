// deliver-step.mjs, the host side of the native omni-deliver workflow, run as
// a child process the way a courier runs it: in a real temp git repository,
// with a fake omni first on PATH (fixtures/fake-omni.mjs) whose state is a
// JSON file.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { fnv1a } from "./deliver-step.mjs";

const helper = fileURLToPath(new URL("./deliver-step.mjs", import.meta.url));
const fakeOmni = fileURLToPath(new URL("./fixtures/fake-omni.mjs", import.meta.url));

const git = (cwd, ...argv) => execFileSync("git", argv, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });

function liveProcess(t) {
  const child = spawn("sleep", ["60"], { stdio: "ignore" });
  t.after(() => child.kill());
  return child.pid;
}

function deadPid() {
  return spawnSync("true").pid;
}

// A checkout `app` with one commit, the fake omni, and a live process standing
// in for the Claude Code session whose pid the lock holds.
function scratch(t, stories = [], threads = [], extra = {}) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "deliver-step-")));
  const repo = join(root, "app");
  mkdirSync(repo);
  git(repo, "init", "-q", "-b", "main");
  git(repo, "config", "user.email", "test@example.com");
  git(repo, "config", "user.name", "Test");
  writeFileSync(join(repo, ".gitignore"), ".omni/\n");
  writeFileSync(join(repo, "README"), "app\n");
  git(repo, "add", ".gitignore", "README");
  git(repo, "commit", "-q", "-m", "init");

  const bin = join(root, "bin");
  mkdirSync(bin);
  writeFileSync(join(bin, "omni"), `#!/bin/sh\nexec "${process.execPath}" "${fakeOmni}" "$@"\n`);
  chmodSync(join(bin, "omni"), 0o755);
  const statePath = join(root, "omni-state.json");
  writeFileSync(statePath, JSON.stringify({ stories, threads, calls: [], checks: { fast: "true", stack_up: "true", stack: "true" }, ...extra }));
  t.after(() => rmSync(root, { recursive: true, force: true }));

  const session = liveProcess(t);
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, FAKE_OMNI_STATE: statePath, OMNI_DELIVER_SESSION_PID: String(session) };
  for (const name of ["FAKE_OMNI_FAIL", "FAKE_OMNI_DOWN", "FAKE_OMNI_HANG"]) delete env[name];
  const state = () => JSON.parse(readFileSync(statePath, "utf8"));
  return {
    root,
    repo,
    env,
    session,
    lock: join(repo, ".omni", "run.lock"),
    lane: join(root, "app-stories", "_lane"),
    state,
    edit: (change) => {
      const s = state();
      change(s);
      writeFileSync(statePath, JSON.stringify(s));
    },
    story: (key) => state().stories.find((s) => s.key === key),
    // The story writes sent to the board, as "<verb> <key>".
    writes: () => state().calls.filter((c) => c.argv[0] === "story" && !["list", "read"].includes(c.argv[1])).map((c) => `${c.argv[1]} ${c.argv[2]}`),
  };
}

function story(key, fields = {}) {
  return { key, title: `Story ${key}`, text: "", criteria: [], status: "backlog", blocked: false, holder: null, version: 1, decisions: [], links: [], blocked_by: [], files_planned: [], ...fields };
}

// A thread on the feature as `omni thread read` shows it; conclusion null when open.
function thread(id, title, conclusion = null) {
  return { id, title, feature: "board", posts: [], conclusion };
}

const questionLink = (id) => ({ thread_id: id, label: "question" });
const sha = (dirs, rev) => git(dirs.repo, "rev-parse", rev).trim();
const ago = (ms) => new Date(Date.now() - ms).toISOString();
const readLock = (dirs) => JSON.parse(readFileSync(dirs.lock, "utf8"));
// A body without the payload's hash.
const pick = ({ received, ...rest }) => rest;

function writeLock(dirs, lock) {
  mkdirSync(dirname(dirs.lock), { recursive: true });
  writeFileSync(dirs.lock, JSON.stringify(lock) + "\n");
}

function commit(cwd, file) {
  writeFileSync(join(cwd, file), `${file}\n`);
  git(cwd, "add", file);
  git(cwd, "commit", "-q", "-m", `feat: ${file}`);
  return git(cwd, "rev-parse", "HEAD").trim();
}

// A commit on the branch that the checkout never sees, as a story the lane landed.
function landOn(dirs, branch, file) {
  if (!git(dirs.repo, "branch", "--list", branch).trim()) git(dirs.repo, "branch", branch);
  const wt = join(dirs.root, `land-${file}`);
  git(dirs.repo, "worktree", "add", "-q", wt, branch);
  commit(wt, file);
  git(dirs.repo, "worktree", "remove", wt);
  return sha(dirs, `refs/heads/${branch}`);
}

// Runs the helper in the checkout as a courier's shell does, and returns the
// body of its output after checking the envelope: exit 0, nothing on standard
// error, one line, and the hash of the body.
function helperRun(dirs, argv, env = {}) {
  const r = spawnSync(process.execPath, [helper, ...argv], { cwd: dirs.repo, env: { ...dirs.env, ...env }, encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(r.stderr, "");
  assert.match(r.stdout, /^[^\n]+\n$/);
  const out = JSON.parse(r.stdout);
  assert.equal(out.hash, fnv1a(JSON.stringify(out.body)));
  return out.body;
}

function start(dirs, { feature = "board", op = "start-1", force = false } = {}) {
  return helperRun(dirs, ["start", "--op", op, "--feature", feature, ...(force ? ["--force"] : [])]);
}

function payloadPath(dirs, run, opId) {
  const file = join(dirs.repo, ".omni", "runs", run, "in", `${opId}.json`);
  mkdirSync(dirname(file), { recursive: true });
  return file;
}

let ops = 0;
// A step of the run, with its payload written where a courier writes it; the
// body must have received the payload's hash.
function step(dirs, run, name, fields = {}) {
  const obj = { opId: `${name}-${++ops}`, ...fields };
  const payload = JSON.stringify(obj);
  const file = payloadPath(dirs, run, obj.opId);
  writeFileSync(file, payload);
  const body = helperRun(dirs, [name, "--run", run, "--payload", file]);
  assert.equal(body.received, fnv1a(payload));
  return body;
}

// ---- the courier protocol -------------------------------------------------------

test("fnv1a is the 32-bit FNV-1a of the string's UTF-16 code units, as 8 lowercase hex digits", () => {
  assert.equal(fnv1a(""), "811c9dc5");
  assert.equal(fnv1a("a"), "e40c292c");
  assert.equal(fnv1a("foobar"), "bf9cf968");
  // Past ASCII it hashes code units, not UTF-8 bytes: a reference in BigInt.
  const reference = (s) => {
    let h = 2166136261n;
    for (let i = 0; i < s.length; i++) h = ((h ^ BigInt(s.charCodeAt(i))) * 16777619n) & 0xffffffffn;
    return h.toString(16).padStart(8, "0");
  };
  const leadingZero = Array.from({ length: 1000 }, (_, i) => `k${i}`).find((s) => reference(s).startsWith("0"));
  for (const s of ["é", "日本語", "😀 grinning", "line\r\nnext", leadingZero]) assert.equal(fnv1a(s), reference(s), s);
});

test("a step reads its payload file as UTF-8, removes one trailing newline, deletes the file, and answers received: the FNV-1a of the payload text", (t) => {
  const dirs = scratch(t);
  const { run } = start(dirs);
  const payload = JSON.stringify({ opId: "list-1", note: "naïve 😀 \"quoted\"" });
  const receivedFor = (text) => {
    const file = payloadPath(dirs, run, "list-1");
    writeFileSync(file, text);
    const body = helperRun(dirs, ["list", "--run", run, "--payload", file]);
    assert.equal(existsSync(file), false, "the payload file is deleted");
    return body.received;
  };

  assert.equal(receivedFor(payload), fnv1a(payload));
  assert.equal(receivedFor(`${payload}\n`), fnv1a(payload));
  assert.equal(receivedFor(`${payload}\n\n`), fnv1a(`${payload}\n`));
});

test("every call prints one line, JSON.stringify({body, hash}) with the FNV-1a of JSON.stringify(body), and exits 0, also when the step fails or the call is wrong", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const file = payloadPath(dirs, run, "claim-1");
  writeFileSync(file, JSON.stringify({ opId: "claim-1", key: "US-1", stage: "review" }));

  for (const argv of [["claim", "--run", run, "--payload", file], ["no-such-step"], ["list", "--run", run, "--payload", join(dirs.repo, "elsewhere.json")]]) {
    const r = spawnSync(process.execPath, [helper, ...argv], { cwd: dirs.repo, env: dirs.env, encoding: "utf8" });
    assert.equal(r.status, 0);
    assert.equal(r.stderr, "");
    assert.match(r.stdout, /^[^\n]+\n$/);
    const line = r.stdout.trimEnd();
    const out = JSON.parse(line);
    assert.equal(line, JSON.stringify({ body: out.body, hash: fnv1a(JSON.stringify(out.body)) }));
    assert.equal(out.body.ok, false);
    assert.equal(typeof out.body.error, "string");
  }
});

test("a write repeated with its opId is answered from .omni/runs/<run>/applied.json and writes nothing again; a new opId writes", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const decision = { opId: "decide-US-1-1", key: "US-1", question: "Which colour is the badge?", choice: "Red", by: "agent" };
  const question = { opId: "thread-US-1-1", key: "US-1", title: "US-1: Which size is the badge?", body: "Options:\n1. Small\n2. Large\n" };

  const first = [step(dirs, run, "decision", decision), step(dirs, run, "thread-open", question)];
  const again = [step(dirs, run, "decision", decision), step(dirs, run, "thread-open", question)];

  assert.deepEqual(again, first);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which colour is the badge?", choice: "Red", by: "agent" }]);
  assert.equal(dirs.state().threads.length, 1);
  const applied = JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json"), "utf8"));
  assert.deepEqual([applied["decide-US-1-1"], applied["thread-US-1-1"]], first);
  step(dirs, run, "decision", { ...decision, opId: "decide-US-1-2" });
  assert.equal(dirs.story("US-1").decisions.length, 2);
});

test("a write waits to record its op while another call holds the record's lock, and takes over a lock left for more than 10 s", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const lock = join(dirs.repo, ".omni", "runs", run, "applied.json.lock");
  const decision = (opId) => {
    const file = payloadPath(dirs, run, opId);
    writeFileSync(file, JSON.stringify({ opId, key: "US-1", question: `Question ${opId}?`, choice: "A", by: "agent" }));
    return file;
  };
  writeFileSync(lock, "");

  const child = spawn(process.execPath, [helper, "decision", "--run", run, "--payload", decision("decide-1")], { cwd: dirs.repo, env: dirs.env });
  let out = "";
  child.stdout.on("data", (d) => { out += d; });
  const closed = new Promise((resolve) => child.on("close", resolve));
  await new Promise((resolve) => setTimeout(resolve, 2000));
  const whileHeld = out;
  rmSync(lock);
  await closed;
  writeFileSync(lock, "");
  const old = new Date(Date.now() - 20000);
  utimesSync(lock, old, old);
  const started = Date.now();
  const late = helperRun(dirs, ["decision", "--run", run, "--payload", decision("decide-2")]);

  assert.equal(whileHeld, "", "no answer while the lock is held");
  assert.equal(JSON.parse(out).body.ok, true);
  assert.equal(late.ok, true);
  assert.ok(Date.now() - started < 10000, "the stale lock was taken over at once");
  assert.equal(existsSync(lock), false);
  assert.deepEqual(Object.keys(JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json"), "utf8"))), ["start-1", "decide-1", "decide-2"]);
});

test("each call with --run refreshes the lock's heartbeat, and leaves a lock that another run holds as it is", (t) => {
  const dirs = scratch(t);
  const { run } = start(dirs);
  const old = { ...readLock(dirs), heartbeat: "2026-01-01T00:00:00.000Z" };
  writeLock(dirs, old);
  const before = Date.now();

  step(dirs, run, "list");

  const lock = readLock(dirs);
  assert.deepEqual({ ...lock, heartbeat: old.heartbeat }, old);
  assert.ok(Date.parse(lock.heartbeat) >= before, `${lock.heartbeat} is the time of the call`);
  const other = { ...lock, run: "run-20261107-120000-1" };
  writeLock(dirs, other);
  step(dirs, run, "list");
  assert.deepEqual(readLock(dirs), other);
});

test("the helper runs through a symlink to it, as ~/.claude/omni/deliver-step.mjs links it", (t) => {
  const dirs = scratch(t);
  const link = join(dirs.root, "omni", "deliver-step.mjs");
  mkdirSync(dirname(link));
  symlinkSync(helper, link);

  const r = spawnSync(process.execPath, [link, "start", "--op", "start-1", "--feature", "board"], { cwd: dirs.repo, env: dirs.env, encoding: "utf8" });

  assert.equal(JSON.parse(r.stdout).body.ok, true);
});

// ---- start: the lock --------------------------------------------------------------

test("start takes .omni/run.lock: the run, the feature, the session's pid, the op, and a heartbeat that starts at the run's start; received is the hash of {opId, feature, force}", (t) => {
  const dirs = scratch(t);

  const body = start(dirs, { op: "start-7" });

  assert.equal(body.ok, true);
  assert.match(body.run, /^run-\d{8}-\d{6}-\d+$/);
  const lock = readLock(dirs);
  assert.deepEqual({ ...lock, started: null, heartbeat: null }, { run: body.run, feature: "board", pid: dirs.session, op: "start-7", started: null, heartbeat: null });
  assert.equal(lock.heartbeat, lock.started);
  assert.ok(Date.now() - Date.parse(lock.started) < 60000);
  assert.equal(body.received, fnv1a(JSON.stringify({ opId: "start-7", feature: "board", force: false })));
});

test("start names the run by the local time of its start", (t) => {
  const dirs = scratch(t);

  const body = helperRun(dirs, ["start", "--op", "start-1", "--feature", "board"], { TZ: "Asia/Tokyo" });

  // Tokyo is UTC+9 all year.
  const tokyo = new Date(Date.parse(readLock(dirs).started) + 9 * 3600000);
  const p = (n) => String(n).padStart(2, "0");
  const local = `${tokyo.getUTCFullYear()}${p(tokyo.getUTCMonth() + 1)}${p(tokyo.getUTCDate())}-${p(tokyo.getUTCHours())}${p(tokyo.getUTCMinutes())}${p(tokyo.getUTCSeconds())}`;
  assert.match(body.run, new RegExp(`^run-${local}-\\d+$`));
});

test("a lock whose session lives and whose heartbeat is under 2 hours old refuses the run, naming it and its feature, and nothing is touched", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } })]);
  const held = { run: "run-20261107-090000-1", feature: "billing", pid: liveProcess(t), op: "start-1", started: ago(3 * 3600000), heartbeat: ago(90 * 60000) };
  writeLock(dirs, held);

  const body = start(dirs);

  assert.equal(body.ok, false);
  assert.match(body.error, /^the tree is held by run run-20261107-090000-1 for feature billing, .*start with force if that run is dead$/);
  assert.deepEqual(readLock(dirs), held);
  assert.deepEqual(dirs.state().calls, []);
  assert.equal(git(dirs.repo, "branch", "--list", "deliver/*"), "");
});

test("a lock whose session has ended is taken over, and the start logs it", (t) => {
  const dirs = scratch(t);
  writeLock(dirs, { run: "run-old", feature: "billing", pid: deadPid(), op: "start-1", started: ago(60000), heartbeat: ago(60000) });

  const body = start(dirs);

  assert.equal(body.ok, true);
  assert.equal(readLock(dirs).run, body.run);
  assert.ok(body.log.some((l) => /^Took over the lock of run run-old for feature billing: its session \(pid \d+\) has ended$/.test(l)), body.log.join("\n"));
});

test("a lock whose heartbeat is more than 2 hours old is taken over while its session still lives, and the start logs it", (t) => {
  const dirs = scratch(t);
  writeLock(dirs, { run: "run-old", feature: "billing", pid: liveProcess(t), op: "start-1", started: ago(5 * 3600000), heartbeat: ago(2 * 3600000 + 60000) });

  const body = start(dirs);

  assert.equal(body.ok, true);
  assert.equal(readLock(dirs).run, body.run);
  assert.ok(body.log.some((l) => /^Took over the lock of run run-old for feature billing: its last heartbeat, \S+, is more than 2 hours old$/.test(l)), body.log.join("\n"));
});

test("start with force takes over a live lock", (t) => {
  const dirs = scratch(t);
  writeLock(dirs, { run: "run-old", feature: "billing", pid: liveProcess(t), op: "start-1", started: ago(60000), heartbeat: ago(60000) });

  const body = start(dirs, { force: true });

  assert.equal(body.ok, true);
  assert.equal(readLock(dirs).run, body.run);
  assert.ok(body.log.includes("Took over the lock of run run-old for feature billing: start with force"), body.log.join("\n"));
  assert.equal(body.received, fnv1a(JSON.stringify({ opId: "start-1", feature: "board", force: true })));
});

test("the same start again from the same session, before its run made any call, gets the first start's body back; once the run has called, it is refused", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } })]);

  const first = start(dirs);
  const again = start(dirs);
  step(dirs, first.run, "list");
  const late = start(dirs);

  assert.deepEqual(again, first);
  assert.deepEqual(dirs.writes(), ["release US-1"]);
  assert.equal(late.ok, false);
  assert.match(late.error, new RegExp(`^the tree is held by run ${first.run} for feature board`));
});

test("without OMNI_DELIVER_SESSION_PID the lock holds the nearest ancestor that is not a shell: the process that ran the helper, directly or through sh", (t) => {
  const dirs = scratch(t);
  const env = { ...dirs.env };
  delete env.OMNI_DELIVER_SESSION_PID;

  const direct = spawnSync(process.execPath, [helper, "start", "--op", "start-1", "--feature", "board"], { cwd: dirs.repo, env, encoding: "utf8" });
  const directPid = readLock(dirs).pid;
  rmSync(dirs.lock);
  // `; exit $?` keeps sh from running node with exec, so sh is node's parent.
  const viaShell = spawnSync("/bin/sh", ["-c", `"${process.execPath}" "${helper}" start --op start-2 --feature board; exit $?`], { cwd: dirs.repo, env, encoding: "utf8" });

  assert.deepEqual([JSON.parse(direct.stdout).body.ok, JSON.parse(viaShell.stdout).body.ok], [true, true]);
  assert.deepEqual([directPid, readLock(dirs).pid], [process.pid, process.pid]);
});

// ---- start: ignored files, the deliver branch, the checks, the lane -----------------

test("start adds .omni/ to .git/info/exclude when nothing ignores it, and only once", (t) => {
  const dirs = scratch(t);
  git(dirs.repo, "rm", "-q", ".gitignore");
  git(dirs.repo, "commit", "-q", "-m", "no ignores");

  start(dirs);
  start(dirs, { op: "start-2", force: true });

  const exclude = readFileSync(join(dirs.repo, ".git", "info", "exclude"), "utf8");
  assert.equal(exclude.split("\n").filter((l) => l === ".omni/").length, 1);
  assert.equal(git(dirs.repo, "status", "--porcelain"), "");
});

test("start leaves .git/info/exclude alone when the checkout already ignores .omni/", (t) => {
  const dirs = scratch(t);
  const before = readFileSync(join(dirs.repo, ".git", "info", "exclude"), "utf8");

  start(dirs);

  assert.equal(readFileSync(join(dirs.repo, ".git", "info", "exclude"), "utf8"), before);
});

test("start creates deliver/<slug> at the checkout's HEAD when it is missing, the slug being the feature lowercased with each non-alphanumeric a dash; the checkout's branch and files stay as they were", (t) => {
  const dirs = scratch(t);
  writeFileSync(join(dirs.repo, "README"), "edited, not committed\n");
  const head = sha(dirs, "HEAD");

  const body = start(dirs, { feature: "Link a Folder: v2" });

  assert.equal(body.branch, "deliver/link-a-folder--v2");
  assert.deepEqual([sha(dirs, "refs/heads/deliver/link-a-folder--v2"), body.tip], [head, head]);
  assert.deepEqual([git(dirs.repo, "rev-parse", "--abbrev-ref", "HEAD").trim(), sha(dirs, "HEAD")], ["main", head]);
  assert.equal(git(dirs.repo, "status", "--porcelain"), " M README\n");
  assert.ok(body.log.includes(`deliver/link-a-folder--v2 created at ${head.slice(0, 7)}, the checkout's HEAD`), body.log.join("\n"));
});

test("start fast-forwards deliver/<slug> to the checkout's HEAD when HEAD is ahead of it", (t) => {
  const dirs = scratch(t);
  const old = sha(dirs, "HEAD");
  git(dirs.repo, "branch", "deliver/board");
  const head = commit(dirs.repo, "main.rs");

  const body = start(dirs);

  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), body.tip], [head, head]);
  assert.ok(body.log.includes(`deliver/board fast-forwarded from ${old.slice(0, 7)} to ${head.slice(0, 7)}, the checkout's HEAD`), body.log.join("\n"));
});

test("start keeps deliver/<slug> where it is when it is ahead of the checkout's HEAD", (t) => {
  const dirs = scratch(t);
  const head = sha(dirs, "HEAD");
  const landed = landOn(dirs, "deliver/board", "landed.rs");

  const body = start(dirs);

  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), body.tip, sha(dirs, "HEAD")], [landed, landed, head]);
  assert.ok(body.log.includes(`deliver/board kept at ${landed.slice(0, 7)}, ahead of the checkout's HEAD ${head.slice(0, 7)}`), body.log.join("\n"));
});

test("start stops with 'merge first' when deliver/<slug> and the checkout's HEAD have diverged, before any work: no lock left, nothing asked of the board, the branch untouched and no lane", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } })]);
  const landed = landOn(dirs, "deliver/board", "landed.rs");
  commit(dirs.repo, "main.rs");

  const body = start(dirs);

  assert.equal(body.ok, false);
  assert.equal(body.error, "deliver/board and HEAD have diverged: merge first (git merge deliver/board), then start again");
  assert.equal(existsSync(dirs.lock), false);
  assert.deepEqual(dirs.state().calls, []);
  assert.equal(sha(dirs, "refs/heads/deliver/board"), landed);
  assert.equal(existsSync(dirs.lane), false);
});

test("start gives the checks fast, stack_up, stack, seed and lint as omni checks has them, each an empty string when unset", (t) => {
  const dirs = scratch(t, [], [], { checks: { fast: "make fast", stack: "make stack", seed: "make seed NAME=x" } });

  const body = start(dirs);

  assert.deepEqual(body.checks, { fast: "make fast", stack_up: "", stack: "make stack", seed: "make seed NAME=x", lint: "" });
});

test("a start whose checks cannot be read stops before any work and leaves no lock", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } })], [], { checks: null });

  const body = start(dirs);

  assert.equal(body.ok, false);
  assert.match(body.error, /^the checks could not be read: /);
  assert.equal(existsSync(dirs.lock), false);
  assert.deepEqual(dirs.writes(), []);
  assert.equal(git(dirs.repo, "branch", "--list", "deliver/*"), "");
});

test("start creates the lane worktree <stories dir>/_lane, detached at the tip of deliver/<slug>, when it is missing", (t) => {
  const dirs = scratch(t);
  const landed = landOn(dirs, "deliver/board", "landed.rs");

  const body = start(dirs);

  assert.equal(body.lane, dirs.lane);
  assert.deepEqual([git(dirs.lane, "rev-parse", "HEAD").trim(), git(dirs.lane, "rev-parse", "--abbrev-ref", "HEAD").trim()], [landed, "HEAD"]);
});

test("start cleans an existing lane worktree with reset --hard and clean -fd: edits and untracked files go, ignored files and its HEAD stay", (t) => {
  const dirs = scratch(t);
  writeFileSync(join(dirs.repo, ".gitignore"), ".omni/\ntarget/\n");
  git(dirs.repo, "commit", "-q", "-am", "ignore target");
  start(dirs);
  const head = git(dirs.lane, "rev-parse", "HEAD").trim();
  writeFileSync(join(dirs.lane, "README"), "edited\n");
  mkdirSync(join(dirs.lane, "stray"));
  writeFileSync(join(dirs.lane, "stray", "notes"), "untracked\n");
  mkdirSync(join(dirs.lane, "target"));
  writeFileSync(join(dirs.lane, "target", "build.out"), "warm cache\n");

  start(dirs, { op: "start-2", force: true });

  assert.equal(readFileSync(join(dirs.lane, "README"), "utf8"), "app\n");
  assert.equal(existsSync(join(dirs.lane, "stray")), false);
  assert.equal(readFileSync(join(dirs.lane, "target", "build.out"), "utf8"), "warm cache\n");
  assert.equal(git(dirs.lane, "rev-parse", "HEAD").trim(), head);
});

test("a directory in the lane worktree's place that is not a worktree of the checkout stops the start, and is left as it is", (t) => {
  const dirs = scratch(t);
  mkdirSync(dirs.lane, { recursive: true });
  writeFileSync(join(dirs.lane, "notes"), "mine\n");

  const body = start(dirs);

  assert.equal(body.ok, false);
  assert.equal(body.error, `${dirs.lane} is not a worktree of ${dirs.repo}, so it cannot be the lane worktree; move it away`);
  assert.equal(readFileSync(join(dirs.lane, "notes"), "utf8"), "mine\n");
  assert.equal(existsSync(dirs.lock), false);
});

// ---- start: reconcile ---------------------------------------------------------------

test("start releases the stories another run holds, and the working ones nobody holds, to their waiting states, keeping an implementing story's worktree, and lists them and the feature's stories", (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } }),
    story("US-2", { status: "testing", holder: { run: "run-old", stage: "testing" } }),
    story("US-3", { status: "implementing", holder: { run: "run-old", stage: "implementing" } }),
    story("US-4", { status: "review" }),
    story("US-5", { status: "implemented" }),
  ]);
  const wt = join(dirs.root, "app-stories", "US-3");
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-3", wt);
  commit(wt, "work.rs");

  const body = start(dirs);

  assert.deepEqual(body.released, [{ key: "US-1", status: "implemented" }, { key: "US-2", status: "reviewed" }, { key: "US-3", status: "backlog" }, { key: "US-4", status: "implemented" }]);
  assert.deepEqual(dirs.state().calls.filter((c) => c.argv[1] === "release").map((c) => c.argv[c.argv.indexOf("--run") + 1]), [body.run, body.run, body.run, body.run]);
  assert.deepEqual(body.stories.map((s) => [s.key, s.status, s.holder]), [["US-1", "implemented", null], ["US-2", "reviewed", null], ["US-3", "backlog", null], ["US-4", "implemented", null], ["US-5", "implemented", null]]);
  assert.equal(readFileSync(join(wt, "work.rs"), "utf8"), "work.rs\n");
});

// ---- the board steps ------------------------------------------------------------------

test("list gives each story's key, title, status, priority, blocked and holder, and the keys the board calls pickable", (t) => {
  const dirs = scratch(t, [story("US-1", { priority: 1 }), story("US-2", { blocked: true }), story("US-3", { status: "implemented" })]);
  const { run } = start(dirs);

  const body = step(dirs, run, "list");

  assert.deepEqual(body.stories, [
    { key: "US-1", title: "Story US-1", status: "backlog", priority: 1, blocked: false, holder: null },
    { key: "US-2", title: "Story US-2", status: "backlog", priority: 3, blocked: true, holder: null },
    { key: "US-3", title: "Story US-3", status: "implemented", priority: 3, blocked: false, holder: null },
  ]);
  assert.deepEqual(body.pickable, ["US-1"]);
});

test("context gives the story's fields and decisions, its open findings by attempt with the numbers taken off and those of no attempt apart, and its linked docs once each with their nodes, never their text", (t) => {
  const docs = { "doc-spec": '<p data-omni-id="n1" data-rev="1">SPEC NODE ONE</p><p data-omni-id="n2" data-rev="1">SPEC NODE TWO</p>', "doc-ui": '<h1 data-omni-id="u1">UI NODE</h1>' };
  const dirs = scratch(t, [
    story("US-1", { id: "id-1", status: "done" }),
    story("US-2", {
      id: "id-2",
      text: "Show a badge.",
      criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }],
      files_planned: ["src/badge.rs"],
      blocked_by: ["id-1"],
      decisions: [{ question: "Which red?", choice: "#c00", by: "human" }],
      links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }, questionLink("th-1"), { doc_id: "doc-ui", node_id: "u1", label: "" }, { doc_id: "doc-spec", node_id: "n2", label: "counts" }],
      findings: [
        { stage: "review", text: "[attempt 1] An old one, answered", files: [], resolved: true },
        { stage: "review", text: "[attempt 1] The badge is blue", files: ["src/badge.rs"], resolved: false },
        { stage: "testing", text: "[attempt 2] The count is missing", files: [], resolved: false },
        { stage: "review", text: "[attempt 1] It has no border", files: [], resolved: false },
        { stage: "testing", text: "rebase conflict in README: rebase it", files: ["README"], resolved: false },
      ],
    }),
  ], [], { docs });
  const { run } = start(dirs);

  const body = step(dirs, run, "context", { key: "US-2" });

  assert.deepEqual(body.story, { key: "US-2", title: "Story US-2", status: "backlog", priority: 3, blocked: false, text: "Show a badge.", criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }], files_planned: ["src/badge.rs"], blocked_by: ["US-1"] });
  assert.deepEqual(body.decisions, [{ question: "Which red?", choice: "#c00", by: "human" }]);
  assert.equal(body.attempt, 2);
  assert.deepEqual(body.findings, [
    { attempt: 1, findings: [{ stage: "review", text: "The badge is blue", files: ["src/badge.rs"] }, { stage: "review", text: "It has no border", files: [] }] },
    { attempt: 2, findings: [{ stage: "testing", text: "The count is missing", files: [] }] },
  ]);
  assert.deepEqual(body.uncounted, [{ stage: "testing", text: "rebase conflict in README: rebase it", files: ["README"] }]);
  assert.deepEqual(body.docs, [{ doc: "doc-spec", nodes: [{ node: "n1", label: "spec" }, { node: "n2", label: "counts" }] }, { doc: "doc-ui", nodes: [{ node: "u1", label: "" }] }]);
  assert.deepEqual(body.threads, ["th-1"]);
  assert.doesNotMatch(JSON.stringify(body), /NODE/);
});

test("claim, move, block, unblock and release write the story as this run and give back its status; a claim the board refuses is ok: false with the board's reason", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { status: "implemented" })]);
  const { run } = start(dirs);
  const state = (body) => [body.ok, body.status, body.blocked, body.holder];

  assert.deepEqual(state(step(dirs, run, "claim", { key: "US-1", stage: "implementing" })), [true, "implementing", false, run]);
  assert.deepEqual(state(step(dirs, run, "move", { key: "US-1", to: "implemented" })), [true, "implemented", false, null]);
  assert.deepEqual(state(step(dirs, run, "claim", { key: "US-1", stage: "review" })), [true, "review", false, run]);
  assert.deepEqual(state(step(dirs, run, "block", { key: "US-1" })), [true, "implemented", true, null]);
  assert.deepEqual(state(step(dirs, run, "unblock", { key: "US-1" })), [true, "implemented", false, null]);
  assert.deepEqual(state(step(dirs, run, "claim", { key: "US-1", stage: "review" })), [true, "review", false, run]);
  assert.deepEqual(state(step(dirs, run, "release", { key: "US-1" })), [true, "implemented", false, null]);
  const refused = step(dirs, run, "claim", { key: "US-2", stage: "testing" });

  assert.equal(refused.ok, false);
  assert.match(refused.error, /^not_waiting: story US-2 is implemented/);
  const writes = dirs.state().calls.filter((c) => ["claim", "move", "block", "unblock"].includes(c.argv[1]));
  assert.deepEqual([...new Set(writes.map((c) => c.argv[c.argv.indexOf("--run") + 1]))], [run]);
});

test("finding numbers the findings of a failed attempt one past the attempts the open findings show, gives a failure that uses no attempt no number, and refuses an attempt that counts without a finding", (t) => {
  const earlier = [{ stage: "review", text: "[attempt 1] The badge is missing", files: [], resolved: false }, { stage: "review", text: "[attempt 4] Answered long ago", files: [], resolved: true }];
  const dirs = scratch(t, [story("US-1", { findings: earlier })]);
  const { run } = start(dirs);

  const counted = step(dirs, run, "finding", { key: "US-1", stage: "review", findings: [{ text: "The badge is blue", files: ["src/badge.rs"] }, { text: "It has no border" }], counts: true });
  const uncounted = step(dirs, run, "finding", { key: "US-1", stage: "testing", findings: [{ text: "rebase conflict in README", files: ["README"] }], counts: false });
  const empty = step(dirs, run, "finding", { key: "US-1", stage: "review", findings: [] });

  assert.deepEqual([counted.attempt, uncounted.attempt], [2, 2]);
  assert.deepEqual(dirs.story("US-1").findings.slice(2), [
    { stage: "review", text: "[attempt 2] The badge is blue", files: ["src/badge.rs"], resolved: false },
    { stage: "review", text: "[attempt 2] It has no border", files: [], resolved: false },
    { stage: "testing", text: "rebase conflict in README", files: ["README"], resolved: false },
  ]);
  assert.equal(empty.ok, false);
  assert.equal(dirs.story("US-1").findings.length, 5);
});

test("files sets the story's files_changed to what its branch changed since it left deliver/<slug>, not what landed there since", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  commit(wt, "board.rs");
  landOn(dirs, "deliver/board", "landed.rs");

  const body = step(dirs, run, "files", { key: "US-1" });

  assert.deepEqual(body.files, ["board.rs"]);
  assert.deepEqual(dirs.story("US-1").files_changed, ["board.rs"]);
});

test("thread-open creates a thread on the feature with its body posted from a file that is gone after; given a key, it links the thread from the story, keeping its links, and blocks it", (t) => {
  const dirs = scratch(t, [story("US-1", { links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }] })]);
  const { run } = start(dirs);
  step(dirs, run, "claim", { key: "US-1", stage: "implementing" });

  const plain = step(dirs, run, "thread-open", { title: "Run question", body: "Plain\n" });
  const linked = step(dirs, run, "thread-open", { title: "US-1: Which colour is the badge?", body: "Options:\n1. Paint it red\n2. Paint it blue\n", key: "US-1" });

  assert.deepEqual(dirs.state().threads.map((th) => [th.id, th.title, th.feature, th.posts]), [
    [plain.thread, "Run question", "board", [{ summary: "Run question", body: "Plain\n" }]],
    [linked.thread, "US-1: Which colour is the badge?", "board", [{ summary: "US-1: Which colour is the badge?", body: "Options:\n1. Paint it red\n2. Paint it blue\n" }]],
  ]);
  for (const c of dirs.state().calls.filter((c) => c.argv[0] === "post")) assert.equal(existsSync(c.argv[c.argv.indexOf("--body-file") + 1]), false);
  assert.deepEqual(dirs.story("US-1").links, [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }, questionLink(linked.thread)]);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").blocked, dirs.story("US-1").holder], ["backlog", true, null]);
  assert.equal(linked.key, "US-1");
});

// ---- unblock-pass ---------------------------------------------------------------------

test("unblock-pass records a concluded thread's conclusion as a human decision with the thread's question, unblocks the story, and gives its key", (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A")]);
  const { run } = start(dirs);

  const body = step(dirs, run, "unblock-pass");

  assert.deepEqual(body.unblocked, ["US-5"]);
  assert.equal(dirs.story("US-5").blocked, false);
  assert.deepEqual(dirs.story("US-5").decisions, [{ question: "US-5: Which colour is the badge?", choice: "Use option A", by: "human" }]);
  assert.deepEqual(dirs.story("US-5").links, [questionLink("th-1")]);
});

test("unblock-pass keeps a story blocked, with no decision, while one of its threads has no conclusion", (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1"), questionLink("th-2")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A"), thread("th-2", "US-5: Which size is the badge?")]);
  const { run } = start(dirs);

  const body = step(dirs, run, "unblock-pass");

  assert.deepEqual(body.unblocked, []);
  assert.equal(dirs.story("US-5").blocked, true);
  assert.deepEqual(dirs.story("US-5").decisions, []);
});

test("unblock-pass records only the conclusions that are not human decisions yet", (t) => {
  const earlier = { question: "US-5: Which colour is the badge?", choice: "Use option A", by: "human" };
  const dirs = scratch(t, [story("US-5", { blocked: true, decisions: [earlier], links: [questionLink("th-1"), questionLink("th-2")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A"), thread("th-2", "US-5: Which size is the badge?", "Use option B")]);
  const { run } = start(dirs);

  step(dirs, run, "unblock-pass");

  assert.deepEqual(dirs.story("US-5").decisions, [earlier, { question: "US-5: Which size is the badge?", choice: "Use option B", by: "human" }]);
  assert.equal(dirs.story("US-5").blocked, false);
});

test("an answer restarts the attempt budget: unblock-pass resolves the story's open findings, context counts no attempt, and the next three failed attempts are 1, 2 and 3", (t) => {
  const findings = [1, 2, 3].map((n) => ({ stage: "review", text: `[attempt ${n}] Still blue`, files: [], resolved: false }));
  const dirs = scratch(t, [story("US-1", { blocked: true, findings, links: [questionLink("th-1")] })], [thread("th-1", "US-1: still failing after 3 attempts", "Red is #c00; try again")]);
  const { run } = start(dirs);

  step(dirs, run, "unblock-pass");
  const context = step(dirs, run, "context", { key: "US-1" });
  const attempts = [1, 2, 3].map(() => step(dirs, run, "finding", { key: "US-1", stage: "review", findings: [{ text: "Still not red" }] }).attempt);

  assert.deepEqual([context.attempt, context.findings], [0, []]);
  assert.deepEqual(attempts, [1, 2, 3]);
  assert.deepEqual(dirs.story("US-1").findings.map((f) => [f.text, f.resolved]), [
    ["[attempt 1] Still blue", true],
    ["[attempt 2] Still blue", true],
    ["[attempt 3] Still blue", true],
    ["[attempt 1] Still not red", false],
    ["[attempt 2] Still not red", false],
    ["[attempt 3] Still not red", false],
  ]);
});

// ---- worktrees and waits ------------------------------------------------------------

test("a new story worktree is cut from the tip of deliver/<slug>, not the checkout's HEAD, on the branch story/<key>", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const landed = landOn(dirs, "deliver/board", "landed.rs");
  const { run } = start(dirs);

  const body = step(dirs, run, "worktree", { key: "US-1" });

  const wt = join(dirs.root, "app-stories", "US-1");
  assert.deepEqual(pick(body), { ok: true, key: "US-1", path: wt, branch: "story/US-1", created: true });
  assert.deepEqual([git(wt, "rev-parse", "HEAD").trim(), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim()], [landed, "story/US-1"]);
});

test("an existing story worktree is kept, with its commits and what it left uncommitted", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  const head = commit(wt, "half.rs");
  writeFileSync(join(wt, "wip.rs"), "wip\n");
  landOn(dirs, "deliver/board", "landed.rs");

  const body = step(dirs, run, "worktree", { key: "US-1" });

  assert.equal(body.created, false);
  assert.equal(git(wt, "rev-parse", "HEAD").trim(), head);
  assert.equal(readFileSync(join(wt, "wip.rs"), "utf8"), "wip\n");
});

test("sleep waits the seconds it is given", (t) => {
  const dirs = scratch(t);
  const { run } = start(dirs);
  const before = Date.now();

  const body = step(dirs, run, "sleep", { seconds: 1 });

  assert.equal(body.seconds, 1);
  assert.ok(Date.now() - before >= 1000);
});

// ---- the clarity cache ------------------------------------------------------------------

test("clarity-pending lists the backlog stories that are not blocked and have no check recorded; once clarity-record records one it is not pending, in a later run too", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3", { blocked: true }), story("US-4", { status: "implemented" })]);
  const { run } = start(dirs);

  const before = step(dirs, run, "clarity-pending").keys;
  const record = step(dirs, run, "clarity-record", { key: "US-1" });
  const after = step(dirs, run, "clarity-pending").keys;
  const next = start(dirs, { op: "start-2", force: true });

  assert.deepEqual([before, after], [["US-1", "US-2"], ["US-2"]]);
  assert.deepEqual(JSON.parse(readFileSync(join(dirs.repo, ".omni", "clarity.json"), "utf8")), { "US-1": record.hash });
  assert.deepEqual(step(dirs, next.run, "clarity-pending").keys, ["US-2"]);
});

test("a story's clarity check is pending again when its text, criteria, decisions or linked node text change, all of the node; not when its title or another node of the doc changes", (t) => {
  // The linked node n1 holds a node of the same element, and text after it.
  const doc = ({ inner = "Badges are red.", tail = "They count.", other = "Unrelated." } = {}) =>
    `<div data-omni-id="n1" data-rev="1"><div data-omni-id="n1a" data-rev="1">${inner}</div><p data-omni-id="n1b" data-rev="1">${tail}</p></div><p data-omni-id="n2" data-rev="1">${other}</p>`;
  const dirs = scratch(t, [story("US-1", { text: "Show a badge.", criteria: [{ text: "The badge is red" }], links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }] })], [], { docs: { "doc-spec": doc() } });
  const { run } = start(dirs);
  const pendingAfter = (change) => {
    step(dirs, run, "clarity-record", { key: "US-1" });
    dirs.edit(change);
    return step(dirs, run, "clarity-pending").keys;
  };
  const us1 = (change) => (state) => change(state.stories.find((s) => s.key === "US-1"));
  const spec = (parts) => (state) => { state.docs["doc-spec"] = doc(parts); };

  assert.deepEqual(pendingAfter(us1((s) => { s.title = "A renamed story"; })), []);
  assert.deepEqual(pendingAfter(spec({ other: "Changed elsewhere." })), []);
  assert.deepEqual(pendingAfter(us1((s) => { s.text = "Show a red badge."; })), ["US-1"]);
  assert.deepEqual(pendingAfter(us1((s) => { s.criteria.push({ text: "The badge counts" }); })), ["US-1"]);
  assert.deepEqual(pendingAfter(us1((s) => { s.decisions.push({ question: "Which red?", choice: "#c00", by: "human" }); })), ["US-1"]);
  assert.deepEqual(pendingAfter(spec({ other: "Changed elsewhere.", inner: "Badges are dark red." })), ["US-1"]);
  assert.deepEqual(pendingAfter(spec({ other: "Changed elsewhere.", inner: "Badges are dark red.", tail: "They do not count." })), ["US-1"]);
});
