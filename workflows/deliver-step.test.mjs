// deliver-step.mjs, the host side of the native omni-deliver workflow, run as
// a child process the way a courier runs it: in a real temp git repository,
// with a fake omni first on PATH (fixtures/fake-omni.mjs) whose state is a
// JSON file.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, renameSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
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
  // A job a test left running is stopped, with its supervisor, before its folders go.
  t.after(() => {
    for (const run of existsSync(join(repo, ".omni", "runs")) ? readdirSync(join(repo, ".omni", "runs")) : []) {
      const dir = join(repo, ".omni", "runs", run, "jobs");
      for (const name of existsSync(dir) ? readdirSync(dir).filter((n) => n.endsWith(".job.json")) : []) {
        const job = JSON.parse(readFileSync(join(dir, name), "utf8"));
        if (existsSync(join(dir, name.replace(/\.job\.json$/, ".json")))) continue;
        for (const pid of [job.pgid > 0 ? -job.pgid : null, job.supervisor > 0 ? job.supervisor : null]) {
          if (Number.isInteger(pid)) try { process.kill(pid, "SIGKILL"); } catch {}
        }
      }
    }
  });
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
// The threads on the board besides the Run threads that start opens.
const questions = (dirs) => dirs.state().threads.filter((th) => !/^Run run-\d{8}-\d{6}-\d+$/.test(th.title));
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

// start's flags as the command carries them, with the hash of its payload object.
function startArgv({ feature = "board", op = "start-1", force = false } = {}) {
  return ["start", "--op", op, "--feature", feature, ...(force ? ["--force"] : []), "--hash", fnv1a(JSON.stringify({ opId: op, feature, force }))];
}

function start(dirs, options = {}) {
  return helperRun(dirs, startArgv(options));
}

function payloadPath(dirs, run, opId) {
  const file = join(dirs.repo, ".omni", "runs", run, "in", `${opId}.json`);
  mkdirSync(dirname(file), { recursive: true });
  return file;
}

// A step's command as a courier runs it, after writing the payload where it
// goes; the command carries the payload's hash.
function stepArgv(dirs, run, name, obj) {
  const payload = JSON.stringify(obj);
  const file = payloadPath(dirs, run, obj.opId);
  writeFileSync(file, payload);
  return [name, "--run", run, "--payload", file, "--hash", fnv1a(payload)];
}

let ops = 0;
// A step of the run, with its payload written where a courier writes it; the
// body must have received the payload's hash. fields.opId sets the opId.
function step(dirs, run, name, fields = {}, env = {}) {
  const obj = { opId: `${name}-${++ops}`, ...fields };
  const body = helperRun(dirs, stepArgv(dirs, run, name, obj), env);
  assert.equal(body.received, fnv1a(JSON.stringify(obj)));
  return body;
}

const appliedOf = (dirs, run) => JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json"), "utf8"));
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const runFile = (dirs, run, name) => join(dirs.repo, ".omni", "runs", run, name);
const jobOf = (dirs, run, id) => JSON.parse(readFileSync(runFile(dirs, run, join("jobs", `${id}.job.json`)), "utf8"));
// True once no process has that pid, or with a negative pid, that group.
const gone = (pid) => { try { process.kill(pid, 0); return false; } catch (e) { return e.code === "ESRCH"; } };
async function eventually(check, ms = 5000) {
  for (const end = Date.now() + ms; !check(); await pause(50)) if (Date.now() > end) return false;
  return true;
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
    const body = helperRun(dirs, ["list", "--run", run, "--payload", file, "--hash", fnv1a(text.replace(/\n$/, ""))]);
    assert.equal(body.ok, true);
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
  const claim = stepArgv(dirs, run, "claim", { opId: "claim-1", key: "US-1", stage: "review" });

  for (const argv of [claim, ["no-such-step"], ["list", "--run", run, "--payload", join(dirs.repo, "elsewhere.json"), "--hash", "811c9dc5"]]) {
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
  assert.equal(questions(dirs).length, 1);
  const applied = JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json"), "utf8"));
  assert.deepEqual([applied["decide-US-1-1"], applied["thread-US-1-1"]], first);
  step(dirs, run, "decision", { ...decision, opId: "decide-US-1-2" });
  assert.equal(dirs.story("US-1").decisions.length, 2);
});

test("every write repeated with its opId gets its first body back from the record and calls nothing again", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-2: Which red?", "#c00")]);
  const { run } = start(dirs);
  const writes = [
    ["claim", { key: "US-1", stage: "implementing" }],
    ["worktree", { key: "US-1" }],
    ["files", { key: "US-1" }],
    ["finding", { key: "US-1", stage: "review", findings: [{ text: "The badge is blue" }] }],
    ["decision", { key: "US-1", question: "Which red?", choice: "#c00", by: "agent" }],
    ["move", { key: "US-1", to: "implemented" }],
    ["claim", { key: "US-1", stage: "review" }],
    ["block", { key: "US-1" }],
    ["unblock", { key: "US-1" }],
    ["claim", { key: "US-1", stage: "review" }],
    ["release", { key: "US-1" }],
    ["thread-open", { title: "Run question", body: "Plain\n" }],
    ["unblock-pass", {}],
    ["clarity-record", { key: "US-1" }],
  ];

  for (const [i, [name, fields]] of writes.entries()) {
    const obj = { opId: `${name}-${i}`, ...fields };
    const first = step(dirs, run, name, obj);
    if (name === "worktree") commit(first.path, "badge.rs");
    const calls = dirs.state().calls.length;
    const again = step(dirs, run, name, obj);

    assert.equal(first.ok, true, `${name}: ${first.error}`);
    assert.deepEqual(again, first, name);
    assert.equal(dirs.state().calls.length, calls, `${name} called nothing again`);
  }
  assert.deepEqual(dirs.story("US-1").files_changed, ["badge.rs"]);
  assert.deepEqual(dirs.state().threads.map((th) => th.title), ["US-2: Which red?", `Run ${run}`, "Run question"]);
});

test("a payload that does not hash to the command's --hash is refused before anything, even for an applied op: nothing is written or recorded; start checks its flags the same way", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const sent = { opId: "decide-1", key: "US-1", question: "Which red?", choice: "#c00", by: "agent" };
  const copied = { ...sent, choice: "#c0O" };
  // The courier wrote `obj`; the command carries the hash of what the script sent.
  const asSent = (obj) => helperRun(dirs, [...stepArgv(dirs, run, "decision", obj).slice(0, -1), fnv1a(JSON.stringify(sent))]);

  const refused = asSent(copied);
  const recorded = appliedOf(dirs, run)["decide-1"];
  const applied = asSent(sent);
  const late = asSent(copied);
  const badStart = helperRun(dirs, [...startArgv({ op: "start-2", force: true }).slice(0, -1), fnv1a("{}")]);

  assert.deepEqual([refused.ok, refused.received, recorded], [false, fnv1a(JSON.stringify(copied)), undefined]);
  assert.equal(refused.error, `the payload hashes to ${fnv1a(JSON.stringify(copied))}, not ${fnv1a(JSON.stringify(sent))}: write it again exactly`);
  assert.deepEqual([applied.ok, late.ok, late.error], [true, false, refused.error]);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which red?", choice: "#c00", by: "agent" }]);
  assert.deepEqual([badStart.ok, readLock(dirs).run], [false, run]);
  assert.match(badStart.error, /^the flags hash to [0-9a-f]{8}, not [0-9a-f]{8}: run the command again exactly$/);
});

test("a repeat of an applied opId with another payload is an error, not a replay, and writes nothing", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const decision = { opId: "decide-1", key: "US-1", question: "Which red?", choice: "#c00", by: "agent" };

  const first = step(dirs, run, "decision", decision);
  const other = step(dirs, run, "decision", { ...decision, choice: "#f00" });

  assert.equal(first.ok, true);
  assert.deepEqual([other.ok, other.error], [false, `the op decide-1 was applied with another payload, which hashed to ${first.received}; a new payload needs a new opId`]);
  assert.equal(dirs.story("US-1").decisions.length, 1);
});

// Runs a step as a courier's helper does, and kills that helper once `made`
// sees the step's first write on the board. FAKE_OMNI_HANG_AFTER holds the
// write's omni call after the fake saved it, so the write is made and its
// answer never comes back, as when a courier's Bash call is cut off.
async function cutOff(t, dirs, run, name, obj, made, env = {}) {
  const child = spawn(process.execPath, [helper, ...stepArgv(dirs, run, name, obj)], { cwd: dirs.repo, env: { ...dirs.env, ...env }, detached: true, stdio: "ignore" });
  const exited = new Promise((resolve) => child.on("exit", resolve));
  t.after(() => { try { process.kill(-child.pid, "SIGKILL"); } catch {} });
  const seen = () => { try { return made(); } catch { return false; } };
  for (const until = Date.now() + 10000; !seen(); await pause(20)) assert.ok(Date.now() < until, `${name} made no write to be cut off after`);
  process.kill(-child.pid, "SIGKILL");
  await exited;
}

// A git hook in the checkout, whatever core.hooksPath the machine sets.
function hook(dirs, name, script) {
  const dir = join(dirs.repo, ".git", "hooks");
  git(dirs.repo, "config", "core.hooksPath", dir);
  writeFileSync(join(dir, name), `#!/bin/sh\n${script}\n`);
  chmodSync(join(dir, name), 0o755);
  return join(dir, name);
}

test("a write whose helper was cut off after its board write left a pending marker, and its repeat carries it on without writing twice: claim, release, move, block, unblock, decision and finding", async (t) => {
  const dirs = scratch(t, ["US-1", "US-2", "US-3", "US-4", "US-5", "US-6", "US-7"].map((key) => story(key)));
  const { run } = start(dirs);
  const held = { run, stage: "implementing" };
  dirs.edit((s) => {
    const at = (key) => s.stories.find((x) => x.key === key);
    Object.assign(at("US-2"), { status: "implementing", holder: held });
    Object.assign(at("US-3"), { status: "implementing", holder: held });
    at("US-5").blocked = true;
  });
  const cases = [
    ["claim", { key: "US-1", stage: "implementing" }, (s) => s.status === "implementing"],
    ["release", { key: "US-2" }, (s) => s.holder === null],
    ["move", { key: "US-3", to: "implemented" }, (s) => s.status === "implemented"],
    ["block", { key: "US-4" }, (s) => s.blocked],
    ["unblock", { key: "US-5" }, (s) => !s.blocked],
    ["decision", { key: "US-6", question: "Which red?", choice: "#c00", by: "agent" }, (s) => s.decisions.length === 1],
    ["finding", { key: "US-7", stage: "review", findings: [{ text: "First" }, { text: "Second" }] }, (s) => (s.findings || []).length === 1],
  ];

  for (const [name, fields, made] of cases) {
    const obj = { opId: `${name}-cut`, ...fields };
    await cutOff(t, dirs, run, name, obj, () => made(dirs.story(fields.key)), { FAKE_OMNI_HANG_AFTER: `story ${name} ${fields.key}` });
    const marker = appliedOf(dirs, run)[obj.opId];
    const again = step(dirs, run, name, obj);

    assert.equal(marker.pending, true, `${name} left a pending marker`);
    assert.equal(again.ok, true, `${name}: ${again.error}`);
    assert.deepEqual(appliedOf(dirs, run)[obj.opId], again, `${name} recorded its body`);
    assert.equal(dirs.state().calls.filter((c) => c.argv[1] === name && c.argv[2] === fields.key).length, name === "finding" ? 2 : 1, `${name} wrote each thing once`);
  }
  assert.deepEqual(dirs.story("US-1").holder, held);
  assert.deepEqual(dirs.story("US-6").decisions, [{ question: "Which red?", choice: "#c00", by: "agent" }]);
  assert.deepEqual(dirs.story("US-7").findings.map((f) => f.text), ["[attempt 1] First", "[attempt 1] Second"]);
});

test("a claim cut off before its write is made by its repeat", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const obj = { opId: "claim-cut", key: "US-1", stage: "implementing" };

  await cutOff(t, dirs, run, "claim", obj, () => appliedOf(dirs, run)["claim-cut"].pending, { FAKE_OMNI_HANG: "story claim US-1" });
  const before = dirs.story("US-1").status;
  const again = step(dirs, run, "claim", obj);

  assert.equal(before, "backlog");
  assert.deepEqual([again.ok, again.status, again.holder], [true, "implementing", run]);
});

test("a thread-open cut off after its post, or after its link, is carried on by its repeat: one thread with one post, linked from the story, which is blocked", async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const { run } = start(dirs);
  const titled = (title) => dirs.state().threads.filter((th) => th.title === title);

  for (const [key, words, made] of [
    ["US-1", "post", (title) => titled(title).some((th) => th.posts.length === 1)],
    ["US-2", "story edit US-2", () => dirs.story("US-2").links.length === 1],
  ]) {
    const obj = { opId: `thread-${key}`, key, title: `${key}: Which colour is the badge?`, body: "Options:\n1. Red\n2. Blue\n" };
    await cutOff(t, dirs, run, "thread-open", obj, () => made(obj.title), { FAKE_OMNI_HANG_AFTER: words });
    const again = step(dirs, run, "thread-open", obj);

    const threads = titled(obj.title);
    assert.equal(threads.length, 1, `${key}: one thread`);
    assert.equal(threads[0].posts.length, 1, `${key}: one post`);
    assert.deepEqual([again.ok, again.thread, dirs.story(key).links, dirs.story(key).blocked], [true, threads[0].id, [questionLink(threads[0].id)], true]);
  }
});

test("a worktree step cut off after it made the worktree is carried on by its repeat, which keeps that worktree and answers created", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = join(dirs.root, "app-stories", "US-1");
  // git worktree add runs post-checkout once the worktree is there.
  const held = hook(dirs, "post-checkout", "sleep 30");

  await cutOff(t, dirs, run, "worktree", { opId: "wt-cut", key: "US-1" }, () => existsSync(join(wt, "README")));
  rmSync(held);
  const again = step(dirs, run, "worktree", { opId: "wt-cut", key: "US-1" });

  assert.deepEqual(pick(again), { ok: true, key: "US-1", path: wt, branch: "story/US-1", created: true });
  assert.equal(git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), "story/US-1");
});

test("a write waits to record its op while another call holds the record's lock, and takes over a lock left for more than 10 s", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const lock = join(dirs.repo, ".omni", "runs", run, "applied.json.lock");
  const decision = (opId) => stepArgv(dirs, run, "decision", { opId, key: "US-1", question: `Question ${opId}?`, choice: "A", by: "agent" });
  writeFileSync(lock, "");

  const child = spawn(process.execPath, [helper, ...decision("decide-1")], { cwd: dirs.repo, env: dirs.env });
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
  const late = helperRun(dirs, decision("decide-2"));

  assert.equal(whileHeld, "", "no answer while the lock is held");
  assert.equal(JSON.parse(out).body.ok, true);
  assert.equal(late.ok, true);
  assert.ok(Date.now() - started < 10000, "the stale lock was taken over at once");
  assert.equal(existsSync(lock), false);
  assert.deepEqual(Object.keys(JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json"), "utf8"))), ["start-1", "decide-1", "decide-2"]);
});

test("each call with --run refreshes the lock's heartbeat; once another run holds the lock, every step of the run fails, a write writes nothing, and the lock stays as it is", (t) => {
  const dirs = scratch(t, [story("US-1")]);
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
  const lost = [step(dirs, run, "list"), step(dirs, run, "claim", { opId: "claim-late", key: "US-1", stage: "implementing" })];
  assert.deepEqual(lost.map((b) => [b.ok, b.error]), [[false, `run ${run} no longer holds the lock`], [false, `run ${run} no longer holds the lock`]]);
  assert.deepEqual(readLock(dirs), other);
  assert.deepEqual(dirs.writes(), []);
  assert.equal(appliedOf(dirs, run)["claim-late"], undefined);
});

test("the helper runs through a symlink to it, as ~/.claude/omni/deliver-step.mjs links it", (t) => {
  const dirs = scratch(t);
  const link = join(dirs.root, "omni", "deliver-step.mjs");
  mkdirSync(dirname(link));
  symlinkSync(helper, link);

  const r = spawnSync(process.execPath, [link, ...startArgv()], { cwd: dirs.repo, env: dirs.env, encoding: "utf8" });

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

  const body = helperRun(dirs, startArgv(), { TZ: "Asia/Tokyo" });

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

test("the same start from the same session is answered from its record only within 5 minutes of it, and never with force, which starts a new run with received of {force: true}", (t) => {
  const dirs = scratch(t);
  const first = start(dirs);
  const sixMinutes = ago(6 * 60000);
  writeLock(dirs, { ...readLock(dirs), started: sixMinutes, heartbeat: sixMinutes });

  const later = start(dirs);
  const forced = start(dirs, { force: true });
  const forcedAgain = start(dirs, { force: true });
  const fiveHours = ago(5 * 3600000);
  writeLock(dirs, { ...readLock(dirs), started: fiveHours, heartbeat: fiveHours });
  const next = start(dirs);

  assert.equal(later.ok, false);
  assert.match(later.error, new RegExp(`^the tree is held by run ${first.run} for feature board`));
  assert.deepEqual([forced.ok, forcedAgain.ok], [true, true]);
  assert.equal(new Set([first.run, forced.run, forcedAgain.run]).size, 3);
  assert.equal(forced.received, fnv1a(JSON.stringify({ opId: "start-1", feature: "board", force: true })));
  assert.equal(next.ok, true);
  assert.notEqual(next.run, forcedAgain.run);
  assert.equal(readLock(dirs).run, next.run);
});

test("start reads and takes the lock while it holds the lock's own lock: a start waits while another start holds it, then finds that start's lock and is refused", async (t) => {
  const dirs = scratch(t);
  mkdirSync(dirname(dirs.lock), { recursive: true });
  writeFileSync(`${dirs.lock}.lock`, "");
  const child = spawn(process.execPath, [helper, ...startArgv()], { cwd: dirs.repo, env: dirs.env });
  let out = "";
  child.stdout.on("data", (d) => { out += d; });
  const closed = new Promise((resolve) => child.on("close", resolve));
  await pause(1500);
  const whileHeld = out;
  const other = { run: "run-20261107-120000-1", feature: "billing", pid: liveProcess(t), op: "start-1", started: ago(1000), heartbeat: ago(1000) };
  writeLock(dirs, other);
  rmSync(`${dirs.lock}.lock`);
  await closed;

  assert.equal(whileHeld, "", "no answer while the lock's lock is held");
  const body = JSON.parse(out).body;
  assert.equal(body.ok, false);
  assert.match(body.error, /^the tree is held by run run-20261107-120000-1 for feature billing/);
  assert.deepEqual(readLock(dirs), other);
});

test("a start that stops keeps the log of what it did before it stopped", (t) => {
  const dirs = scratch(t);
  writeLock(dirs, { run: "run-old", feature: "billing", pid: deadPid(), op: "start-1", started: ago(60000), heartbeat: ago(60000) });
  landOn(dirs, "deliver/board", "landed.rs");
  commit(dirs.repo, "main.rs");

  const body = start(dirs);

  assert.equal(body.ok, false);
  assert.match(body.error, /^deliver\/board and HEAD have diverged/);
  assert.ok(body.log.some((l) => /^Took over the lock of run run-old for feature billing: its session \(pid \d+\) has ended$/.test(l)), JSON.stringify(body.log));
});

test("without OMNI_DELIVER_SESSION_PID the lock holds the nearest ancestor that is not a shell: the process that ran the helper, directly or through sh", (t) => {
  const dirs = scratch(t);
  const env = { ...dirs.env };
  delete env.OMNI_DELIVER_SESSION_PID;

  const direct = spawnSync(process.execPath, [helper, ...startArgv()], { cwd: dirs.repo, env, encoding: "utf8" });
  const directPid = readLock(dirs).pid;
  rmSync(dirs.lock);
  // `; exit $?` keeps sh from running node with exec, so sh is node's parent.
  const viaShell = spawnSync("/bin/sh", ["-c", `"${process.execPath}" "${helper}" ${startArgv({ op: "start-2" }).join(" ")}; exit $?`], { cwd: dirs.repo, env, encoding: "utf8" });

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

test("a lane folder that lost its .git file inside an enclosing repository stops the start, and neither that repository nor the folder is touched", (t) => {
  const dirs = scratch(t);
  // The checkout sits in a repository of its own, as in a monorepo folder or a home kept in git.
  git(dirs.root, "init", "-q", "-b", "main");
  git(dirs.root, "config", "user.email", "test@example.com");
  git(dirs.root, "config", "user.name", "Test");
  writeFileSync(join(dirs.root, "notes.txt"), "committed\n");
  git(dirs.root, "add", "notes.txt");
  git(dirs.root, "commit", "-q", "-m", "outer");
  start(dirs);
  rmSync(dirs.lock);
  rmSync(join(dirs.lane, ".git"));
  mkdirSync(join(dirs.lane, "target"));
  writeFileSync(join(dirs.lane, "target", "cache.bin"), "warm\n");
  writeFileSync(join(dirs.root, "notes.txt"), "uncommitted work\n");

  const body = start(dirs, { op: "start-2" });

  assert.equal(body.ok, false);
  assert.equal(body.error, `${dirs.lane} is not a worktree of ${dirs.repo}, so it cannot be the lane worktree; move it away`);
  assert.equal(readFileSync(join(dirs.root, "notes.txt"), "utf8"), "uncommitted work\n");
  assert.deepEqual([readFileSync(join(dirs.lane, "target", "cache.bin"), "utf8"), readFileSync(join(dirs.lane, "README"), "utf8")], ["warm\n", "app\n"]);
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

test("context gives the resolved findings of the attempts before the latest answers as earlier, one group per attempt in the order they came", (t) => {
  const finding = (text, resolved, stage = "review") => ({ stage, text, files: [], resolved });
  const dirs = scratch(t, [story("US-1", { findings: [
    finding("[attempt 1] The badge is blue", true),
    finding("[attempt 1] It has no border", true),
    finding("[attempt 2] Still blue", true, "testing"),
    finding("rebase conflict in README", true, "testing"),
    finding("[attempt 1] Too dark a red", true),
    finding("[attempt 1] Now too light", false),
  ] })]);
  const { run } = start(dirs);

  const body = step(dirs, run, "context", { key: "US-1" });

  assert.deepEqual(body.earlier, [
    { attempt: 1, findings: [{ stage: "review", text: "The badge is blue", files: [] }, { stage: "review", text: "It has no border", files: [] }] },
    { attempt: 2, findings: [{ stage: "testing", text: "Still blue", files: [] }] },
    { attempt: 1, findings: [{ stage: "review", text: "Too dark a red", files: [] }] },
  ]);
  assert.deepEqual([body.attempt, body.findings], [1, [{ attempt: 1, findings: [{ stage: "review", text: "Now too light", files: [] }] }]]);
});

test("a story list past 1 MB is read whole", (t) => {
  const dirs = scratch(t, [story("US-1", { text: "x".repeat(2 * 1024 * 1024) })]);
  const { run } = start(dirs);

  const body = step(dirs, run, "list");

  assert.deepEqual(body.stories.map((s) => s.key), ["US-1"]);
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

test("a finding's text loses any attempt number it brings: an uncounted one can never count as an attempt, and a counted one gets only its own", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);

  step(dirs, run, "finding", { key: "US-1", stage: "testing", findings: [{ text: "[attempt 3] rebase conflict in README" }], counts: false });
  const counted = step(dirs, run, "finding", { key: "US-1", stage: "review", findings: [{ text: "[attempt 9] [attempt 8] The badge is blue" }] });

  assert.deepEqual(dirs.story("US-1").findings.map((f) => f.text), ["rebase conflict in README", "[attempt 1] The badge is blue"]);
  assert.equal(counted.attempt, 1);
});

test("files gives a path that is not ASCII as it is, not quoted", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  mkdirSync(join(wt, "docs"));
  commit(wt, "docs/översikt.md");

  const body = step(dirs, run, "files", { key: "US-1" });

  assert.deepEqual(body.files, ["docs/översikt.md"]);
  assert.deepEqual(dirs.story("US-1").files_changed, ["docs/översikt.md"]);
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

  assert.deepEqual(questions(dirs).map((th) => [th.id, th.title, th.feature, th.posts]), [
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

test("unblock-pass keeps a story whose thread cannot be read blocked, says so in its log, and goes on with the next story", (t) => {
  const dirs = scratch(t, [story("US-1", { blocked: true, links: [questionLink("th-gone")] }), story("US-2", { blocked: true, links: [questionLink("th-2")] })], [thread("th-2", "US-2: Which red?", "#c00")]);
  const { run } = start(dirs);

  const body = step(dirs, run, "unblock-pass");

  assert.deepEqual([body.ok, body.unblocked, body.log], [true, ["US-2"], ["US-1 stays blocked: the thread th-gone could not be read: not_found: no thread th-gone in this project"]]);
  assert.deepEqual([dirs.story("US-1").blocked, dirs.story("US-2").blocked], [true, false]);
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

test("worktree keeps an existing folder only when it is a worktree of the checkout on story/<key>: a plain folder or a worktree on another branch is an error, and stays as it is", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const { run } = start(dirs);
  const plain = join(dirs.root, "app-stories", "US-1");
  mkdirSync(plain);
  writeFileSync(join(plain, "notes"), "mine\n");
  const other = join(dirs.root, "app-stories", "US-2");
  git(dirs.repo, "worktree", "add", "-q", "-b", "elsewhere", other);

  const bodies = [step(dirs, run, "worktree", { key: "US-1" }), step(dirs, run, "worktree", { key: "US-2" })];

  assert.deepEqual(bodies.map((b) => [b.ok, b.error]), [
    [false, `${plain} is not a worktree of ${dirs.repo} on story/US-1; move it away`],
    [false, `${other} is not a worktree of ${dirs.repo} on story/US-2; move it away`],
  ]);
  assert.equal(readFileSync(join(plain, "notes"), "utf8"), "mine\n");
  assert.equal(git(other, "rev-parse", "--abbrev-ref", "HEAD").trim(), "elsewhere");
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

// ---- the run's thread, its checks and its timings -------------------------------------

test("start opens the run's thread on the feature, named Run <run> as omni-deliver.js names it, returns it, and keeps it in run.json with the run's start, tip and checks", (t) => {
  const dirs = scratch(t);

  const body = start(dirs);

  assert.deepEqual(dirs.state().threads.map((th) => [th.id, th.title, th.feature, th.posts]), [[body.thread, `Run ${body.run}`, "board", []]]);
  const info = JSON.parse(readFileSync(runFile(dirs, body.run, "run.json"), "utf8"));
  assert.ok(Date.now() - Date.parse(info.started) < 60000);
  assert.deepEqual({ ...info, started: null }, { run: body.run, feature: "board", root: dirs.repo, thread: body.thread, started: null, tip: body.tip, checks: body.checks, landed: [] });
});

test("checks-set merges checks into the run's run.json and gives all five back; an unknown check, or a command that is not a string, is refused", (t) => {
  const dirs = scratch(t, [], [], { checks: { fast: "make fast", stack_up: "make up", stack: "make stack" } });
  const { run } = start(dirs);

  const body = step(dirs, run, "checks-set", { checks: { seed: "make seed NAME=x", lint: "make lint", fast: "make quick" } });
  const unknown = step(dirs, run, "checks-set", { checks: { stackup: "make up" } });
  const odd = step(dirs, run, "checks-set", { checks: { seed: 3 } });

  const checks = { fast: "make quick", stack_up: "make up", stack: "make stack", seed: "make seed NAME=x", lint: "make lint" };
  assert.deepEqual(body.checks, checks);
  assert.deepEqual(JSON.parse(readFileSync(runFile(dirs, run, "run.json"), "utf8")).checks, checks);
  assert.deepEqual([unknown.ok, unknown.error], [false, "no check stackup; the checks are fast, stack_up, stack, seed, lint"]);
  assert.deepEqual([odd.ok, odd.error], [false, "a check is a command, as a string: seed"]);
});

test("every step that runs records its step, op, key, stage, start and end in the run's timings.jsonl; a repeat answered from the record does not", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const before = Date.now();

  step(dirs, run, "list", { opId: "list-x" });
  step(dirs, run, "claim", { opId: "claim-x", key: "US-1", stage: "implementing" });
  step(dirs, run, "claim", { opId: "claim-x", key: "US-1", stage: "implementing" });

  const lines = readFileSync(runFile(dirs, run, "timings.jsonl"), "utf8").trim().split("\n").map((l) => JSON.parse(l));
  assert.deepEqual(lines.map((l) => [l.step, l.op, l.key, l.stage, l.ok]), [["list", "list-x", undefined, undefined, true], ["claim", "claim-x", "US-1", "implementing", true]]);
  for (const l of lines) {
    assert.ok(Date.parse(l.started) >= before - 1000 && Date.parse(l.ended) >= Date.parse(l.started));
    assert.equal(l.ms, Date.parse(l.ended) - Date.parse(l.started));
  }
});

// ---- jobs -------------------------------------------------------------------------------

test("job-start runs the command in a process group of its own, its output going to the job's log; job-wait answers done: false at maxSeconds while it runs, then the result and the log's tail once it ends", (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);

  const started = step(dirs, run, "job-start", { name: "fast", key: "US-1", cmd: 'echo "leader $$ group $(ps -o pgid= -p $$)"; sleep 2; echo done', cwd: lane });
  const begun = Date.now();
  const waiting = step(dirs, run, "job-wait", { id: started.id, maxSeconds: 0.5 });
  const waited = Date.now() - begun;
  const ended = step(dirs, run, "job-wait", { id: started.id, maxSeconds: 30 });

  assert.deepEqual(pick(started), { ok: true, id: "US-1-fast-1", log: runFile(dirs, run, join("jobs", "US-1-fast-1.log")) });
  assert.deepEqual(pick(waiting), { ok: true, id: "US-1-fast-1", done: false });
  assert.ok(waited >= 500 && waited < 1900, `${waited} ms`);
  assert.deepEqual(Object.keys(ended.result).sort(), ["code", "durationMs", "ended", "ok", "signal", "started", "timedOut"]);
  assert.deepEqual([ended.done, ended.result.ok, ended.result.code, ended.result.signal, ended.result.timedOut], [true, true, 0, null, false]);
  assert.ok(ended.result.durationMs >= 2000);
  const [, leader, group] = ended.tail.match(/^leader (\d+) group\s+(\d+)\ndone\n$/);
  assert.equal(leader, group, "the command leads a group of its own");
  assert.equal(readFileSync(started.log, "utf8"), ended.tail);
});

test("a job past its limit gets SIGTERM to its whole group, and its result says it timed out; its log ends naming the job, not its command", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const pidFile = join(dirs.root, "child.pid");

  const { id } = step(dirs, run, "job-start", { name: "stack", cmd: `sleep 30 & echo $! > ${pidFile}; wait`, cwd: lane, limitMinutes: 1 / 60 });
  const body = step(dirs, run, "job-wait", { id, maxSeconds: 30 });

  assert.deepEqual([body.done, body.result.ok, body.result.timedOut, body.result.signal], [true, false, true, "SIGTERM"]);
  assert.ok(body.result.durationMs >= 1000 && body.result.durationMs < 8000, `${body.result.durationMs} ms`);
  assert.ok(await eventually(() => gone(Number(readFileSync(pidFile, "utf8")))), "the command's own child was stopped with it");
  assert.ok(body.tail.endsWith(`\nTimed out: the job ${id} ran past its limit of 1s and was stopped.\n`), body.tail);
  assert.doesNotMatch(body.tail, /sleep 30/);
});

test("the SIGKILL stays armed after the group's leader exits: a member that ignores SIGTERM is killed when the grace is over", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const pidFile = join(dirs.root, "stubborn.pid");
  // The leader waits and dies of SIGTERM; its child ignores SIGTERM and keeps no hold on the log.
  const cmd = `sh -c 'trap "" TERM; echo $$ > ${pidFile}; exec sleep 30' > /dev/null 2>&1 & wait`;

  const { id } = step(dirs, run, "job-start", { name: "stack", cmd, cwd: lane, limitMinutes: 1 / 60 }, { OMNI_DELIVER_KILL_GRACE_MS: "1000" });
  const body = step(dirs, run, "job-wait", { id, maxSeconds: 30 });

  assert.deepEqual([body.done, body.result.timedOut], [true, true]);
  assert.ok(body.result.durationMs >= 2000, `${body.result.durationMs} ms: the result waited for the SIGKILL`);
  assert.ok(await eventually(() => gone(Number(readFileSync(pidFile, "utf8")))), "the member that ignored SIGTERM got SIGKILL");
});

test("a job's limit is rounded to whole milliseconds and kept to 2^31 - 1 ms; none, 0 or less, or not a number means 45 minutes", (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const job = (limitMinutes, cmd = "true") => step(dirs, run, "job-start", { name: "check", cmd, cwd: lane, ...(limitMinutes === undefined ? {} : { limitMinutes }) }).id;

  const huge = job(1e9, "sleep 1");
  const fraction = job(0.02057, "sleep 5");
  const odd = [undefined, 0, -5, "soon", null].map((m) => job(m));
  const results = new Map([huge, fraction, ...odd].map((id) => [id, step(dirs, run, "job-wait", { id, maxSeconds: 30 }).result]));

  assert.deepEqual([huge, fraction, ...odd].map((id) => jobOf(dirs, run, id).limitMs), [2 ** 31 - 1, 1234, ...odd.map(() => 45 * 60000)]);
  assert.deepEqual([results.get(huge).ok, results.get(huge).timedOut], [true, false]);
  assert.deepEqual([results.get(fraction).ok, results.get(fraction).timedOut], [false, true]);
  assert.ok(results.get(fraction).durationMs >= 1234 && results.get(fraction).durationMs < 4000, `${results.get(fraction).durationMs} ms`);
  for (const id of odd) assert.deepEqual([results.get(id).ok, results.get(id).timedOut], [true, false], id);
});

test("a job's output reaches its log byte for byte, so a character split across two writes survives; the tail is the log's last 2 KB from its first whole character", (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  // é (C3 A9) split across two writes, then a thousand three-byte 日 (E6 97 A5).
  const cmd = "printf '\\303'; sleep 0.3; printf '\\251 caf\\303\\251\\n'; i=0; while [ $i -lt 1000 ]; do printf '\\346\\227\\245'; i=$((i+1)); done";

  const { id, log } = step(dirs, run, "job-start", { name: "fast", cmd, cwd: lane });
  const body = step(dirs, run, "job-wait", { id, maxSeconds: 30 });

  assert.equal(readFileSync(log, "utf8"), `é café\n${"日".repeat(1000)}`);
  // 3009 bytes: the last 2048 begin inside a 日, whose two remaining bytes are dropped.
  assert.equal(body.tail, "日".repeat(682));
});

test("job-kill stops a running job's group, and gives its result and the tail that says so; a second kill finds it ended", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const { id } = step(dirs, run, "job-start", { name: "stack", cmd: "sleep 30", cwd: lane });
  const { pgid } = jobOf(dirs, run, id);

  const body = step(dirs, run, "job-kill", { id });
  const again = step(dirs, run, "job-kill", { id });

  assert.deepEqual([body.killed, body.done, body.result.ok, body.result.timedOut, body.result.signal], [true, true, false, false, "SIGTERM"]);
  assert.ok(body.tail.endsWith(`\nStopped: the job ${id} was stopped before it ended.\n`), body.tail);
  assert.ok(await eventually(() => gone(-pgid)));
  assert.deepEqual([again.killed, again.done, again.result], [false, true, body.result]);
});

test("start stops the jobs of earlier runs that still run, found by the pids in their job files, and logs each", async (t) => {
  const dirs = scratch(t);
  const first = start(dirs);
  const { id } = step(dirs, first.run, "job-start", { name: "stack", cmd: "sleep 30", cwd: first.lane });
  const { pgid } = jobOf(dirs, first.run, id);
  // The first run's session ended without a finish.
  writeLock(dirs, { ...readLock(dirs), pid: deadPid() });

  const next = start(dirs, { op: "start-2" });

  assert.equal(next.ok, true);
  assert.ok(next.log.includes(`Stopped the job ${id} of run ${first.run}, which had ended`), next.log.join("\n"));
  assert.ok(await eventually(() => gone(-pgid)));
  const result = JSON.parse(readFileSync(runFile(dirs, first.run, join("jobs", `${id}.json`)), "utf8"));
  assert.deepEqual([result.ok, result.signal], [false, "SIGTERM"]);
});

test("job-wait answers a job whose supervisor is gone without its result as lost", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const { id } = step(dirs, run, "job-start", { name: "stack", cmd: "sleep 30", cwd: lane });
  const job = jobOf(dirs, run, id);
  process.kill(job.supervisor, "SIGKILL");
  await eventually(() => gone(job.supervisor));

  const body = step(dirs, run, "job-wait", { id, maxSeconds: 5 });

  process.kill(-job.pgid, "SIGKILL");
  assert.deepEqual([body.done, body.result.ok, body.result.error], [true, false, `the supervisor of the job ${id} ended without its result`]);
});

// ---- the lane ---------------------------------------------------------------------------

// A story whose worktree has one commit.
function storyWork(dirs, run, key, file = "badge.rs") {
  const wt = step(dirs, run, "worktree", { key }).path;
  commit(wt, file);
  return wt;
}

test("lane-prepare rebases story/<key> onto the tip of deliver/<slug> in its worktree, and checks the candidate out, detached, in a cleaned lane worktree; the checkout is untouched", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const { run, lane } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  const tip = landOn(dirs, "deliver/board", "landed.rs");
  writeFileSync(join(lane, "README"), "edited in the lane\n");
  writeFileSync(join(lane, "stray.txt"), "untracked\n");
  const head = sha(dirs, "HEAD");

  const body = step(dirs, run, "lane-prepare", { key: "US-1" });
  const missingOne = step(dirs, run, "lane-prepare", { key: "US-2" });

  const candidate = git(wt, "rev-parse", "HEAD").trim();
  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "ready", base: tip, candidate });
  assert.deepEqual([sha(dirs, "refs/heads/story/US-1"), git(wt, "rev-parse", "HEAD^").trim()], [candidate, tip]);
  assert.deepEqual([git(lane, "rev-parse", "HEAD").trim(), git(lane, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(lane, "status", "--porcelain")], [candidate, "HEAD", ""]);
  assert.deepEqual([sha(dirs, "HEAD"), git(dirs.repo, "rev-parse", "--abbrev-ref", "HEAD").trim()], [head, "main"]);
  assert.deepEqual(pick(missingOne), { ok: true, key: "US-2", outcome: "missing", reason: `no worktree ${join(dirs.root, "app-stories", "US-2")}` });
});

test("a rebase conflict is an outcome: lane-prepare undoes the rebase and gives the conflicting files, and the story's branch and the lane stay as they were", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run, lane } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  writeFileSync(join(wt, "README"), "the story's README\n");
  git(wt, "commit", "-q", "-am", "feat: README");
  const before = git(wt, "rev-parse", "HEAD").trim();
  landOn(dirs, "deliver/board", "README");
  const laneHead = git(lane, "rev-parse", "HEAD").trim();

  const body = step(dirs, run, "lane-prepare", { key: "US-1" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "conflict", files: ["README"] });
  assert.deepEqual([git(wt, "rev-parse", "HEAD").trim(), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(wt, "status", "--porcelain")], [before, "story/US-1", ""]);
  assert.equal(git(lane, "rev-parse", "HEAD").trim(), laneHead);
});

test("a rebase past its limit is infra, not a conflict: lane-prepare stops it, undoes it, and says so", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  landOn(dirs, "deliver/board", "landed.rs");
  const before = git(wt, "rev-parse", "HEAD").trim();
  hook(dirs, "pre-rebase", "sleep 5");
  const begun = Date.now();

  const body = step(dirs, run, "lane-prepare", { key: "US-1" }, { OMNI_DELIVER_REBASE_MS: "1000" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "infra", reason: "the rebase of story/US-1 onto deliver/board ran past its limit of 1s" });
  assert.ok(Date.now() - begun < 4500, "stopped at its limit, not at the hook's end");
  assert.deepEqual([git(wt, "rev-parse", "HEAD").trim(), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim()], [before, "story/US-1"]);
});

test("lane-land lands the lane's HEAD, the candidate with the test author's commits on it, by a compare-and-swap of deliver/<slug>; the story is done with its files, criteria ticked and findings resolved, and its worktree and branch are gone at once", async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }];
  const dirs = scratch(t, [story("US-1", { status: "reviewed", criteria, findings: [{ stage: "review", text: "[attempt 1] The badge is blue", files: [], resolved: false }] })]);
  const { run, lane } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base, candidate } = step(dirs, run, "lane-prepare", { key: "US-1" });
  const tested = commit(lane, "badge.test");
  const head = sha(dirs, "HEAD");

  const body = step(dirs, run, "lane-land", { key: "US-1", base });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", landed: true, base, final: tested, files: ["badge.rs", "badge.test"], removed: true });
  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), git(dirs.repo, "rev-parse", `${tested}^`).trim()], [tested, candidate]);
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.files_changed], ["done", ["badge.rs", "badge.test"]]);
  assert.deepEqual(done.criteria.map((c) => [c.ticked, !!c.pending]), [[true, false], [true, true]]);
  assert.deepEqual(done.findings.map((f) => f.resolved), [true]);
  assert.deepEqual([existsSync(wt), git(dirs.repo, "branch", "--list", "story/US-1")], [false, ""]);
  assert.doesNotMatch(git(dirs.repo, "worktree", "list", "--porcelain"), /US-1/);
  assert.ok(await eventually(() => readdirSync(join(dirs.root, "app-stories", "_trash")).length === 0), "its files went in the background");
  assert.deepEqual([sha(dirs, "HEAD"), git(dirs.repo, "status", "--porcelain")], [head, ""]);
  assert.deepEqual(JSON.parse(readFileSync(runFile(dirs, run, "run.json"), "utf8")).landed, [{ key: "US-1", base, final: tested }]);
});

test("lane-land refuses a branch that moved since the base: deliver/<slug> stays where it moved to, and the story is not done", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const { run } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  const moved = landOn(dirs, "deliver/board", "other.rs");

  const body = step(dirs, run, "lane-land", { key: "US-1", base });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", landed: false, reason: `deliver/board moved to ${moved.slice(0, 7)} since the base ${base.slice(0, 7)}` });
  assert.equal(sha(dirs, "refs/heads/deliver/board"), moved);
  assert.deepEqual([dirs.story("US-1").status, existsSync(wt)], ["testing", true]);
});

test("lane-land refuses a lane HEAD that does not descend from the base, or does not hold the story's branch, and moves nothing", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  const tip = landOn(dirs, "deliver/board", "landed.rs");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  const story1 = sha(dirs, "refs/heads/story/US-1");

  git(lane, "checkout", "-q", "--detach", base);
  const testsOnly = commit(lane, "only.test");
  const dropped = step(dirs, run, "lane-land", { key: "US-1", base });
  git(lane, "checkout", "-q", "--detach", `${base}^`);
  const offBase = commit(lane, "elsewhere.test");
  const elsewhere = step(dirs, run, "lane-land", { key: "US-1", base });

  assert.deepEqual([dropped.landed, dropped.reason], [false, `the lane's HEAD ${testsOnly.slice(0, 7)} does not hold story/US-1 at ${story1.slice(0, 7)}`]);
  assert.deepEqual([elsewhere.landed, elsewhere.reason], [false, `the lane's HEAD ${offBase.slice(0, 7)} does not descend from the base ${base.slice(0, 7)}`]);
  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), dirs.story("US-1").status], [tip, "testing"]);
});

test("lane-land checks that the landed tree is the tested one: a branch moved again right after the swap is an error, and the story is not done", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const { run } = start(dirs);
  storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base, candidate } = step(dirs, run, "lane-prepare", { key: "US-1" });
  // Another writer puts the branch back at the base as soon as the swap is committed.
  const once = join(dirs.root, "race");
  writeFileSync(once, base);
  hook(dirs, "reference-transaction", [
    '[ "$1" = committed ] || exit 0',
    "while read old new ref; do",
    `  if [ "$ref" = refs/heads/deliver/board ] && [ -f "${once}" ]; then`,
    `    target=$(cat "${once}"); rm "${once}"`,
    '    git update-ref refs/heads/deliver/board "$target"',
    "  fi",
    "done",
  ].join("\n"));

  const body = step(dirs, run, "lane-land", { key: "US-1", base });

  const tree = (rev) => git(dirs.repo, "rev-parse", `${rev}^{tree}`).trim().slice(0, 7);
  assert.deepEqual([body.ok, body.error], [false, `deliver/board holds the tree ${tree(base)} after the landing, not ${tree(candidate)}, the tree of ${candidate.slice(0, 7)} that was tested`]);
  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), dirs.story("US-1").status], [base, "testing"]);
});

test("lane-base and lane-reset put the lane worktree back at the tip of deliver/<slug>, clean and detached", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  const tip = landOn(dirs, "deliver/board", "landed.rs");
  step(dirs, run, "lane-prepare", { key: "US-1" });
  const dirty = () => {
    writeFileSync(join(lane, "README"), "edited\n");
    writeFileSync(join(lane, "stray.txt"), "untracked\n");
  };

  dirty();
  const onBase = step(dirs, run, "lane-base");
  const atBase = [git(lane, "rev-parse", "HEAD").trim(), git(lane, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(lane, "status", "--porcelain")];
  git(lane, "checkout", "-q", "--detach", "refs/heads/story/US-1");
  dirty();
  const reset = step(dirs, run, "lane-reset");

  assert.deepEqual([onBase.tip, reset.tip], [tip, tip]);
  assert.deepEqual(atBase, [tip, "HEAD", ""]);
  assert.deepEqual([git(lane, "rev-parse", "HEAD").trim(), git(lane, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(lane, "status", "--porcelain")], [tip, "HEAD", ""]);
});

// ---- waiting for answers --------------------------------------------------------------

test("wait-answers returns as soon as a blocked story's linked threads all have a conclusion", async (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-5: Which red?")]);
  const { run } = start(dirs);
  const child = spawn(process.execPath, [helper, ...stepArgv(dirs, run, "wait-answers", { opId: "wait-1", maxSeconds: 20 })], { cwd: dirs.repo, env: dirs.env });
  let out = "";
  child.stdout.on("data", (d) => { out += d; });
  const closed = new Promise((resolve) => child.on("close", resolve));
  const begun = Date.now();
  await pause(1000);
  // The human answers. The fake omni writes its whole state on each call, so
  // the answer is written, whole, until the helper has seen it.
  const statePath = dirs.env.FAKE_OMNI_STATE;
  const answer = setInterval(() => {
    try {
      const s = dirs.state();
      if (s.threads.find((th) => th.id === "th-1").conclusion) return;
      s.threads.find((th) => th.id === "th-1").conclusion = "#c00";
      writeFileSync(`${statePath}.tmp`, JSON.stringify(s));
      renameSync(`${statePath}.tmp`, statePath);
    } catch {}
  }, 100);
  await closed;
  clearInterval(answer);

  assert.deepEqual(pick(JSON.parse(out).body), { ok: true, answered: ["US-5"], waiting: [] });
  assert.ok(Date.now() - begun < 6000, `${Date.now() - begun} ms`);
});

test("wait-answers returns at maxSeconds while no answer comes, with the stories that wait; at once when no story waits on a thread", (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1")] }), story("US-6", { blocked: true })], [thread("th-1", "US-5: Which red?")]);
  const { run } = start(dirs);

  let begun = Date.now();
  const late = step(dirs, run, "wait-answers", { maxSeconds: 1 });
  const lateMs = Date.now() - begun;
  dirs.edit((s) => { s.stories[0].links = []; });
  begun = Date.now();
  const none = step(dirs, run, "wait-answers", { maxSeconds: 20 });
  const noneMs = Date.now() - begun;

  assert.deepEqual([late.answered, late.waiting], [[], ["US-5"]]);
  assert.ok(lateMs >= 1000 && lateMs < 4000, `${lateMs} ms`);
  assert.deepEqual([none.answered, none.waiting], [[], []]);
  assert.ok(noneMs < 3000, `${noneMs} ms`);
});

// ---- finish -----------------------------------------------------------------------------

test("finish stops the run's jobs that still run, posts the report on the run's thread from a body file, keeps it as report.md, and gives the lock up", async (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "reviewed", criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }] }),
    story("US-2", { blocked: true, links: [questionLink("th-1")] }),
  ], [thread("th-1", "US-2: Which red?")]);
  const { run, lane, thread: runThread } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  const feat = git(wt, "rev-parse", "HEAD").trim().slice(0, 7);
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  const fast = step(dirs, run, "job-start", { name: "fast", key: "US-1", cmd: "true", cwd: lane });
  step(dirs, run, "job-wait", { id: fast.id, maxSeconds: 30 });
  const tests = commit(lane, "badge.test").slice(0, 7);
  step(dirs, run, "lane-land", { key: "US-1", base });
  const stack = step(dirs, run, "job-start", { name: "stack", key: "US-1", cmd: "sleep 30", cwd: lane });
  const { pgid } = jobOf(dirs, run, stack.id);
  const report = {
    outcomes: [{ key: "US-1", outcome: "done" }, { key: "US-2", outcome: "blocked", detail: "a question for the human" }],
    council: { settled: 2, asked: 1 },
    blocked: [{ key: "US-2", threads: ["th-1"] }],
    journeys: ["healthy: the badge flow works"],
    notes: ["The lane stopped once: the stack did not start."],
  };

  const body = step(dirs, run, "finish", { report });

  assert.deepEqual(pick(body), { ok: true, thread: runThread, posted: true, report: runFile(dirs, run, "report.md"), stopped: [stack.id] });
  assert.ok(await eventually(() => gone(-pgid)), "the running job was stopped");
  assert.equal(existsSync(dirs.lock), false);
  const posts = dirs.state().threads.find((th) => th.id === runThread).posts;
  const text = readFileSync(body.report, "utf8");
  assert.deepEqual(posts, [{ summary: `Report of ${run}`, body: text }]);
  const post = dirs.state().calls.find((c) => c.argv[0] === "post" && c.argv[1] === runThread);
  assert.equal(existsSync(post.argv[post.argv.indexOf("--body-file") + 1]), false, "the body file is gone once posted");
  const [top, timings] = text.split("## Timings\n");
  assert.equal(top, `# Run ${run} on board

The lane stopped once: the stack did not start.

## Landed on deliver/board
- US-1 Story US-1: ${feat} feat: badge.rs; ${tests} feat: badge.test

Bring them into your branch with \`git merge --ff-only deliver/board\`.

## Blocked
- US-2 Story US-2: thread th-1

## Outcomes
- US-1 Story US-1: done
- US-2 Story US-2: blocked: a question for the human

## Council
- 2 questions settled by the council, 1 sent to the human

## Pending human checks
- US-1: It looks right

## Journeys
- healthy: the badge flow works

`);
  assert.match(timings, /^The run took \d+s\.\n\n\| Story \| Total \| implementing \| review \| testing \| rebase \| fast \| stack \|\n\|---\|---\|---\|---\|---\|---\|---\|---\|\n\| US-1 \| \d+s \| - \| - \| \d+s \| \d+s \| \d+s \| \d+s \|\n$/);
});
