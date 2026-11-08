// deliver-step.mjs, the host side of the native omni-deliver workflow, run as
// a child process the way a courier runs it: in a real temp git repository,
// with a fake omni first on PATH (fixtures/fake-omni.mjs) whose state is a
// JSON file.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, symlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { fnv1a, TEST_OBLIGATION, testQuestion, waitSeconds } from "./deliver-step.mjs";

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
  for (const name of ["FAKE_OMNI_FAIL", "FAKE_OMNI_DOWN", "FAKE_OMNI_HANG", "FAKE_OMNI_HANG_AFTER"]) delete env[name];
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

// The record of applied ops, of one step when given.
const appliedOf = (dirs, run, step) => {
  const applied = JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json"), "utf8"));
  return step ? applied[step] || {} : applied;
};
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const runFile = (dirs, run, name) => join(dirs.repo, ".omni", "runs", run, name);
const jobOf = (dirs, run, id) => JSON.parse(readFileSync(runFile(dirs, run, join("jobs", `${id}.job.json`)), "utf8"));
// True once no process has that pid, or with a negative pid, that group.
const gone = (pid) => { try { process.kill(pid, 0); return false; } catch (e) { return e.code === "ESRCH"; } };
// The pid of the process that runs __supervise on this job file, as ps shows it.
const supervisorOf = (jobFile) => {
  const line = execFileSync("ps", ["-ax", "-ww", "-o", "pid=,args="], { encoding: "utf8" }).split("\n").find((l) => l.includes("__supervise") && l.includes(jobFile));
  assert.ok(line, `no supervisor of ${jobFile}`);
  return Number(line.trim().split(/\s+/)[0]);
};
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
  assert.deepEqual([applied.decision["decide-US-1-1"], applied["thread-open"]["thread-US-1-1"]], first);
  step(dirs, run, "decision", { ...decision, opId: "decide-US-1-2", choice: "Blue" });
  assert.equal(dirs.story("US-1").decisions.length, 2);
});

test("a decision the story has already, the same question with the same choice, is not written again under a new opId, whoever took it: the script no longer knows the story's decisions", (t) => {
  const dirs = scratch(t, [story("US-1", { decisions: [{ question: "Which red?", choice: "#c00", by: "human" }] })]);
  const { run } = start(dirs);

  const same = step(dirs, run, "decision", { key: "US-1", question: "Which red?", choice: "#c00", by: "agent" });
  const other = step(dirs, run, "decision", { key: "US-1", question: "Which red?", choice: "#f00", by: "agent" });

  assert.deepEqual([same.recorded, other.recorded], [false, true]);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which red?", choice: "#c00", by: "human" }, { question: "Which red?", choice: "#f00", by: "agent" }]);
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
  const recorded = appliedOf(dirs, run, "decision")["decide-1"];
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

test("the record of applied ops is keyed by step name and opId: an opId one step used is new to another", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);

  const decided = step(dirs, run, "decision", { opId: "op-1", key: "US-1", question: "Which red?", choice: "#c00", by: "agent" });
  const claimed = step(dirs, run, "claim", { opId: "op-1", key: "US-1", stage: "implementing" });

  assert.deepEqual([decided.ok, claimed.ok, claimed.status], [true, true, "implementing"]);
  assert.deepEqual([appliedOf(dirs, run, "decision")["op-1"], appliedOf(dirs, run, "claim")["op-1"]], [decided, claimed]);
});

test("a body with ok false says whether the same command again can work: retry true for flags or a payload that came wrong and for a write that failed, which the repeat carries on; false for a claim the board refuses, a lock another run holds, a worktree on another branch with no story/<key>, a start that must merge first and a payload the step cannot take", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { status: "implemented" }), story("US-3")]);
  const { run } = start(dirs);
  const decision = { opId: "decide-1", key: "US-1", question: "Which red?", choice: "#c00", by: "agent" };
  const question = { opId: "thread-1", title: "Run question", body: "Plain\n" };

  const miscopied = helperRun(dirs, [...stepArgv(dirs, run, "decision", decision).slice(0, -1), "00000000"]);
  const badFlags = helperRun(dirs, [...startArgv({ op: "start-2" }).slice(0, -1), "00000000"]);
  const cutShort = step(dirs, run, "thread-open", question, { FAKE_OMNI_FAIL: "thread" });
  const carried = step(dirs, run, "thread-open", question);
  const notWaiting = step(dirs, run, "claim", { key: "US-2", stage: "testing" });
  git(dirs.repo, "worktree", "add", "-q", "-b", "elsewhere", join(dirs.root, "app-stories", "US-3"));
  const wrongBranch = step(dirs, run, "worktree", { key: "US-3" });
  const unfit = step(dirs, run, "decision", { key: "US-1" });
  writeLock(dirs, { ...readLock(dirs), run: "run-20261107-120000-1" });
  const lost = step(dirs, run, "list");
  const held = start(dirs, { op: "start-3" });
  rmSync(dirs.lock);
  landOn(dirs, "deliver/board", "landed.rs");
  commit(dirs.repo, "main.rs");
  const diverged = start(dirs, { op: "start-4" });

  const flags = (body) => [body.ok, body.retry];
  assert.deepEqual([miscopied, badFlags, cutShort].map(flags), [[false, true], [false, true], [false, true]]);
  assert.deepEqual([carried.ok, questions(dirs).length], [true, 1]);
  assert.deepEqual([notWaiting, wrongBranch, unfit, lost, held, diverged].map(flags), [[false, false], [false, false], [false, false], [false, false], [false, false], [false, false]]);
  assert.match(notWaiting.error, /^not_waiting: /);
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
    const marker = appliedOf(dirs, run, name)[obj.opId];
    const again = step(dirs, run, name, obj);

    assert.equal(marker.pending, true, `${name} left a pending marker`);
    assert.equal(again.ok, true, `${name}: ${again.error}`);
    assert.deepEqual(appliedOf(dirs, run, name)[obj.opId], again, `${name} recorded its body`);
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

  await cutOff(t, dirs, run, "claim", obj, () => appliedOf(dirs, run, "claim")["claim-cut"].pending, { FAKE_OMNI_HANG: "story claim US-1" });
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

test("two thread-opens on one story at once both link it: a story's link writes go one at a time, under a lock of the story", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  // Each read of the story takes a second: thread-opens at once would each
  // link the story from what they read, and the second would drop the first's.
  const env = { ...dirs.env, FAKE_OMNI_HANG_AFTER: "story read US-1", FAKE_OMNI_HANG_MS: "1000" };
  const open = (n) => new Promise((resolve) => {
    const child = spawn(process.execPath, [helper, ...stepArgv(dirs, run, "thread-open", { opId: `thread-${n}`, key: "US-1", title: `US-1: Question ${n}?`, body: `Body ${n}\n` })], { cwd: dirs.repo, env });
    let out = "";
    child.stdout.on("data", (d) => { out += d; });
    child.on("close", () => resolve(JSON.parse(out).body));
  });

  const bodies = await Promise.all([open(1), pause(300).then(() => open(2))]);

  assert.deepEqual(bodies.map((b) => b.ok), [true, true]);
  assert.deepEqual(dirs.story("US-1").links.map((l) => l.thread_id).sort(), bodies.map((b) => b.thread).sort());
  assert.equal(dirs.story("US-1").blocked, true);
});

test("a worktree step cut off after it made the worktree is carried on by its repeat, which keeps that worktree and answers created", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = join(dirs.root, "app-stories", "US-1");
  // git worktree add runs post-checkout once the worktree is there: the hook
  // says so, and holds the step until it is cut off.
  const made = join(dirs.root, "made");
  const held = hook(dirs, "post-checkout", `touch "${made}"; sleep 30`);

  await cutOff(t, dirs, run, "worktree", { opId: "wt-cut", key: "US-1" }, () => existsSync(made));
  rmSync(held);
  const again = step(dirs, run, "worktree", { opId: "wt-cut", key: "US-1" });

  assert.deepEqual(pick(again), { ok: true, key: "US-1", path: wt, branch: "story/US-1", created: true });
  assert.equal(git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), "story/US-1");
});

test("a worktree step cut off while it made the worktree, which it left with a staged deletion, removes that worktree on its repeat and makes it again: one it was making counts only when clean and on story/<key>", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = join(dirs.root, "app-stories", "US-1");
  // A checkout killed half way leaves its files staged as deleted; this hook
  // leaves one so, says so, and holds the step until it is cut off. The test
  // runs no git there meanwhile: a git status takes index.lock, which would
  // fail the hook's git rm.
  const staged = join(dirs.root, "staged");
  const held = hook(dirs, "post-checkout", `git rm -q --cached README && touch "${staged}"; sleep 30`);

  await cutOff(t, dirs, run, "worktree", { opId: "wt-cut", key: "US-1" }, () => existsSync(staged));
  rmSync(held);
  const again = step(dirs, run, "worktree", { opId: "wt-cut", key: "US-1" });

  assert.deepEqual(pick(again), { ok: true, key: "US-1", path: wt, branch: "story/US-1", created: true });
  assert.deepEqual([git(wt, "status", "--porcelain"), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim()], ["", "story/US-1"]);
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
  assert.deepEqual(Object.entries(appliedOf(dirs, run)).map(([name, ops]) => [name, Object.keys(ops)]), [["start", ["start-1"]], ["decision", ["decide-1", "decide-2"]]]);
});

test("a waiter takes a stale lock over only while it holds the lock's takeover guard, after a fresh look under it: while another waiter takes it over it waits, and the lock that waiter has just taken stays", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const lock = join(dirs.repo, ".omni", "runs", run, "applied.json.lock");
  // A lock its holder left 20 s ago, and another waiter in the middle of taking it over.
  writeFileSync(lock, `${deadPid()} 1`);
  const old = new Date(Date.now() - 20000);
  utimesSync(lock, old, old);
  writeFileSync(`${lock}.takeover`, "");
  const child = spawn(process.execPath, [helper, ...stepArgv(dirs, run, "decision", { opId: "decide-1", key: "US-1", question: "Which red?", choice: "#c00", by: "agent" })], { cwd: dirs.repo, env: dirs.env });
  let out = "";
  child.stdout.on("data", (d) => { out += d; });
  const closed = new Promise((resolve) => child.on("close", resolve));
  await pause(1500);
  const whileTaking = out;
  // That waiter has taken the lock, and gives its guard up.
  const taker = liveProcess(t);
  writeFileSync(lock, `${taker} 2`);
  rmSync(`${lock}.takeover`);
  await pause(1500);
  const whileHeld = [out, readFileSync(lock, "utf8")];
  rmSync(lock);
  await closed;

  assert.equal(whileTaking, "", "no answer while another waiter takes the lock over");
  assert.deepEqual(whileHeld, ["", `${taker} 2`], "the lock that waiter took stays, and is waited for");
  assert.equal(JSON.parse(out).body.ok, true);
});

test("a lock whose holder has ended is taken over at once, young as it is", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  writeFileSync(join(dirs.repo, ".omni", "runs", run, "applied.json.lock"), `${deadPid()} 1`);
  const begun = Date.now();

  const body = step(dirs, run, "decision", { key: "US-1", question: "Which red?", choice: "#c00", by: "agent" });

  assert.equal(body.ok, true);
  assert.ok(Date.now() - begun < 5000, `${Date.now() - begun} ms`);
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
  assert.equal(appliedOf(dirs, run, "claim")["claim-late"], undefined);
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

test("the same start from the same session is answered from its record only within 5 minutes of it; a forced start's retry too, which gets the run its first try started instead of taking its lock over", (t) => {
  const dirs = scratch(t);
  const first = start(dirs);
  const sixMinutes = ago(6 * 60000);
  writeLock(dirs, { ...readLock(dirs), started: sixMinutes, heartbeat: sixMinutes });

  const later = start(dirs);
  const forced = start(dirs, { force: true });
  const forcedAgain = start(dirs, { force: true });
  const lockAfter = readLock(dirs);
  const fiveHours = ago(5 * 3600000);
  writeLock(dirs, { ...readLock(dirs), started: fiveHours, heartbeat: fiveHours });
  const next = start(dirs);

  assert.equal(later.ok, false);
  assert.match(later.error, new RegExp(`^the tree is held by run ${first.run} for feature board`));
  assert.deepEqual([forced.ok, forced.received], [true, fnv1a(JSON.stringify({ opId: "start-1", feature: "board", force: true }))]);
  assert.notEqual(forced.run, first.run);
  assert.deepEqual(forcedAgain, forced);
  assert.equal(lockAfter.run, forced.run);
  assert.deepEqual(dirs.state().threads.map((th) => th.title), [`Run ${first.run}`, `Run ${forced.run}`, `Run ${next.run}`]);
  assert.equal(next.ok, true);
  assert.notEqual(next.run, forced.run);
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

test("start gives the checks fast, stack_up, stack, stack_reset, seed and lint as omni checks has them, each an empty string when unset", (t) => {
  const dirs = scratch(t, [], [], { checks: { fast: "make fast", stack: "make stack", stack_reset: "make stack-down", seed: "make seed NAME=x" } });

  const body = start(dirs);

  assert.deepEqual(body.checks, { fast: "make fast", stack_up: "", stack: "make stack", stack_reset: "make stack-down", seed: "make seed NAME=x", lint: "" });
});

test("a start whose checks cannot be read stops before any work and leaves no lock, and is not tried again: a repeat would read the same configuration", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } })], [], { checks: null });

  const body = start(dirs);

  assert.deepEqual([body.ok, body.retry], [false, false]);
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

// root writes anywhere, so a folder it may not write takes another user.
test("a start that cannot write the stories folder beside the checkout stops before any work, and says to add it to the session's working directories", { skip: process.getuid?.() === 0 && "run as root" }, (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } })]);
  const stories = join(dirs.root, "app-stories");
  // Empty, so that the scratch's removal takes it all the same.
  mkdirSync(stories);
  chmodSync(stories, 0o555);

  const body = start(dirs);

  assert.deepEqual([body.ok, body.retry, body.error], [false, false, `${stories} cannot be written (EACCES): add ${stories} to this session's working directories, then start again`]);
  assert.deepEqual([existsSync(dirs.lock), dirs.writes(), dirs.state().threads, git(dirs.repo, "branch", "--list", "deliver/*")], [false, [], [], ""]);
});

// ---- start: reconcile ---------------------------------------------------------------

test("start releases the stories another run holds, and the working ones nobody holds, to their waiting states, keeping an implementing story's worktree, and lists them; of the feature's stories it gives only their keys and counts, since outputs stay small", (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "review", holder: { run: "run-old", stage: "review" } }),
    story("US-2", { status: "testing", holder: { run: "run-old", stage: "testing" } }),
    story("US-3", { status: "implementing", holder: { run: "run-old", stage: "implementing" } }),
    story("US-4", { status: "review" }),
    story("US-5", { status: "implemented", blocked: true }),
  ]);
  const wt = join(dirs.root, "app-stories", "US-3");
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-3", wt);
  commit(wt, "work.rs");

  const body = start(dirs);

  assert.deepEqual(body.released, [{ key: "US-1", status: "implemented" }, { key: "US-2", status: "reviewed" }, { key: "US-3", status: "backlog" }, { key: "US-4", status: "implemented" }]);
  assert.deepEqual(dirs.state().calls.filter((c) => c.argv[1] === "release").map((c) => c.argv[c.argv.indexOf("--run") + 1]), [body.run, body.run, body.run, body.run]);
  assert.deepEqual(body.stories, { keys: ["US-1", "US-2", "US-3", "US-4", "US-5"], counts: { implemented: 3, reviewed: 1, backlog: 1 }, blocked: 1 });
  assert.equal(readFileSync(join(wt, "work.rs"), "utf8"), "work.rs\n");
});

// ---- the board steps ------------------------------------------------------------------

test("list gives, of the other stories, only those the script takes up where they wait, implemented or reviewed and not blocked, as key and status, since outputs stay small, and the keys the board calls pickable", (t) => {
  const dirs = scratch(t, [
    story("US-1", { priority: 1 }),
    story("US-2", { blocked: true }),
    story("US-3", { status: "implemented" }),
    story("US-4", { status: "done" }),
    story("US-5", { status: "cancelled" }),
    story("US-6", { status: "reviewed" }),
    story("US-7", { status: "reviewed", blocked: true }),
  ]);
  const { run } = start(dirs);

  const body = step(dirs, run, "list");

  assert.deepEqual(body.waiting, [{ key: "US-3", status: "implemented" }, { key: "US-6", status: "reviewed" }]);
  assert.deepEqual(body.pickable, ["US-1"]);
  assert.equal(body.stories, undefined);
});

test("list gives the pickable stories with a worktree from an earlier attempt first, then in the board's order: priority, then how many open stories each one unblocks, then number", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { priority: 1 }), story("US-3"), story("US-4"), story("US-5", { blocked_by: ["US-4"] }), story("US-6")]);
  const { run } = start(dirs);
  // US-6 kept the worktree of an attempt that failed; US-1 has a folder that is not a worktree.
  storyWork(dirs, run, "US-6");
  mkdirSync(join(dirs.root, "app-stories", "US-1"));

  const body = step(dirs, run, "list");

  assert.deepEqual(body.pickable, ["US-6", "US-2", "US-4", "US-1", "US-3"]);
});

test("context gives only the facts the script decides by, and no text of the story: the attempt, the status, blocked, clarified, how many criteria, which are test obligations by index, how many findings are open, used no attempt or came before the latest answer, and the fnv1a of each decision's question", (t) => {
  const docs = { "doc-spec": '<p data-omni-id="n1" data-rev="1">SPEC NODE ONE</p>' };
  const dirs = scratch(t, [
    story("US-1", { id: "id-1", status: "done" }),
    story("US-2", {
      id: "id-2",
      status: "implemented",
      blocked: true,
      text: "STORY TEXT: show a badge.",
      criteria: [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the badge", human_check: false }, { text: "It looks right", human_check: true }],
      files_planned: ["src/badge.rs"],
      blocked_by: ["id-1"],
      decisions: [{ question: "Which red?", choice: "#c00", by: "human" }, { question: "Which red?", choice: "#f00", by: "agent" }, { question: "How big?", choice: "16px", by: "agent" }],
      links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }, questionLink("th-1")],
      findings: [
        { stage: "review", text: "[attempt 1] An old one, answered", files: [], resolved: true },
        { stage: "review", text: "[attempt 1] The badge is blue", files: ["src/badge.rs"], resolved: false },
        { stage: "testing", text: "[attempt 2] The count is missing", files: [], resolved: false },
        { stage: "review", text: "[attempt 1] It has no border", files: [], resolved: false },
        { stage: "testing", text: "rebase conflict in README: rebase it", files: ["README"], resolved: false },
        { stage: "testing", text: "an uncounted one, resolved", files: [], resolved: true },
      ],
    }),
  ], [], { docs });
  const { run } = start(dirs);

  const body = step(dirs, run, "context", { key: "US-2" });

  assert.deepEqual(pick(body), {
    ok: true,
    key: "US-2",
    status: "implemented",
    blocked: true,
    attempt: 2,
    clarified: false,
    criteria: 3,
    obligations: [1],
    findings: { open: 3, uncounted: 1, earlier: 1 },
    questions: [fnv1a("Which red?"), fnv1a("How big?")],
  });
  assert.doesNotMatch(JSON.stringify(body), /NODE|STORY TEXT|badge|#c00|border|README|doc-spec|th-1/);
});

test("context gives as test obligations, once clarity has checked the story, only the criteria clarity's decisions name, with the question and choice the script writes; before, a criterion that delivers a test, never one with a bare test or suite in it", (t) => {
  const criteria = [{ text: "The Test connection button shows Connected" }, { text: "A regression test covers the badge" }, { text: "The suite of badges loads" }, { text: "Tests cover the badge count" }];
  const decided = { question: testQuestion("The Test connection button shows Connected"), choice: TEST_OBLIGATION, by: "agent" };
  const dirs = scratch(t, [story("US-1", { criteria }), story("US-2", { criteria, decisions: [decided] })]);
  const { run } = start(dirs);
  mkdirSync(join(dirs.repo, ".omni"), { recursive: true });
  writeFileSync(join(dirs.repo, ".omni", "clarity.json"), JSON.stringify({ "US-2": "0123abcd" }));

  const [unchecked, checked] = ["US-1", "US-2"].map((key) => step(dirs, run, "context", { key }));

  assert.deepEqual([unchecked.clarified, unchecked.obligations], [false, [1, 3]]);
  assert.deepEqual([checked.clarified, checked.obligations], [true, [0]]);
});

test("the helper's test-obligation question and choice are the script's, which records them", () => {
  const source = readFileSync(fileURLToPath(new URL("./omni-deliver.js", import.meta.url)), "utf8");
  const choice = source.match(/^const TEST_OBLIGATION = '((?:[^'\\]|\\.)*)'$/m);
  const question = source.match(/^const testQuestion = \(criterion\) => `(.*)`$/m);

  assert.ok(choice && question, "the script declares TEST_OBLIGATION and testQuestion");
  assert.equal(choice[1].replace(/\\'/g, "'"), TEST_OBLIGATION);
  assert.equal(question[1].replace("${criterion}", "X"), testQuestion("X"));
});

test("exhaust opens a thread named after the story's attempts, with every attempt's findings whole, those that used no attempt and those before the latest answer, read from the board, links it and blocks the story; its answer gives none of them", (t) => {
  const long = `Seen:\n\`\`\`\n${"a".repeat(5000)}\nFINAL-ERROR\n\`\`\``;
  const dirs = scratch(t, [story("US-1", { status: "backlog", findings: [
    { stage: "review", text: "[attempt 1] Before the answer", files: [], resolved: true },
    { stage: "review", text: "[attempt 1] The badge is blue", files: ["src/badge.rs"], resolved: false },
    { stage: "testing", text: `[attempt 2] ${long}`, files: [], resolved: false },
    { stage: "testing", text: "rebase conflict in README", files: ["README"], resolved: false },
    { stage: "review", text: "[attempt 3] Still blue", files: [], resolved: false },
  ] })]);
  const { run } = start(dirs);

  const body = step(dirs, run, "exhaust", { key: "US-1", opId: "exhaust-US-1-1" });
  const again = step(dirs, run, "exhaust", { key: "US-1", opId: "exhaust-US-1-1" });

  const [th] = questions(dirs);
  assert.deepEqual(pick(body), { ok: true, thread: th.id, key: "US-1", attempt: 3 });
  assert.equal(th.title, "US-1: still failing after 3 attempts");
  assert.deepEqual(th.posts.map((p) => p.body), [`The story failed 3 attempts since the human's latest answer. The findings of every attempt:

Attempt 1:
- [review] The badge is blue (src/badge.rs)

Attempt 2:
- [testing] ${long}

Attempt 3:
- [review] Still blue

Open findings that used no attempt:
- [testing] rebase conflict in README (README)

Before the latest answer:

Attempt 1:
- [review] Before the answer
`]);
  assert.deepEqual([dirs.story("US-1").blocked, dirs.story("US-1").links], [true, [questionLink(th.id)]]);
  assert.deepEqual(again, body);
  assert.ok(JSON.stringify(body).length < 300);
});

test("a story list past 1 MB is read whole", (t) => {
  const dirs = scratch(t, [story("US-1", { text: "x".repeat(2 * 1024 * 1024) })]);
  const { run } = start(dirs);

  const body = step(dirs, run, "context", { key: "US-1" });

  assert.equal(body.key, "US-1");
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

test("files records a path that is not ASCII as it is, not quoted, and gives how many files, not their paths", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  mkdirSync(join(wt, "docs"));
  commit(wt, "docs/översikt.md");

  const body = step(dirs, run, "files", { key: "US-1" });

  assert.equal(body.files, 1);
  assert.deepEqual(dirs.story("US-1").files_changed, ["docs/översikt.md"]);
});

test("files sets the story's files_changed to what its branch changed since it left deliver/<slug>, not what landed there since", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  commit(wt, "board.rs");
  landOn(dirs, "deliver/board", "landed.rs");

  const body = step(dirs, run, "files", { key: "US-1" });

  assert.equal(body.files, 1);
  assert.deepEqual(dirs.story("US-1").files_changed, ["board.rs"]);
});

test("thread-open creates a thread on the feature with its body posted from a file in the run's folder that is gone after; given a key, it links the thread from the story, keeping its links, and blocks it", (t) => {
  const dirs = scratch(t, [story("US-1", { links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }] })]);
  const { run } = start(dirs);
  step(dirs, run, "claim", { key: "US-1", stage: "implementing" });

  const plain = step(dirs, run, "thread-open", { title: "Run question", body: "Plain\n" });
  const linked = step(dirs, run, "thread-open", { title: "US-1: Which colour is the badge?", body: "Options:\n1. Paint it red\n2. Paint it blue\n", key: "US-1" });

  assert.deepEqual(questions(dirs).map((th) => [th.id, th.title, th.feature, th.posts]), [
    [plain.thread, "Run question", "board", [{ summary: "Run question", body: "Plain\n" }]],
    [linked.thread, "US-1: Which colour is the badge?", "board", [{ summary: "US-1: Which colour is the badge?", body: "Options:\n1. Paint it red\n2. Paint it blue\n" }]],
  ]);
  for (const c of dirs.state().calls.filter((c) => c.argv[0] === "post")) {
    const file = c.argv[c.argv.indexOf("--body-file") + 1];
    assert.deepEqual([dirname(file), existsSync(file)], [join(dirs.repo, ".omni", "runs", run), false]);
  }
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

test("unblock-pass logs the first 20 stories it keeps blocked on threads it cannot read, and counts the rest, since outputs stay small", (t) => {
  const keys = Array.from({ length: 22 }, (_, i) => `US-${i + 1}`);
  const dirs = scratch(t, keys.map((key) => story(key, { blocked: true, links: [questionLink(`th-${key}`)] })));
  const { run } = start(dirs);

  const body = step(dirs, run, "unblock-pass");

  assert.deepEqual(body.log, [
    ...keys.slice(0, 20).map((key) => `${key} stays blocked: the thread th-${key} could not be read: not_found: no thread th-${key} in this project`),
    "and 2 more stories stay blocked on threads that could not be read",
  ]);
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

  assert.deepEqual([context.attempt, context.findings], [0, { open: 0, uncounted: 0, earlier: 3 }]);
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

test("worktree keeps an existing folder only when it is a worktree of the checkout: a plain folder, or a worktree on another branch with no story/<key> to go back to, is an error, and stays as it is", (t) => {
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

test("a worktree step that finds story/<key> without its worktree makes the worktree again on that branch, so its commits stay, and answers created false: the story has earlier work", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = step(dirs, run, "worktree", { key: "US-1" }).path;
  const work = commit(wt, "half.rs");
  rmSync(wt, { recursive: true, force: true });

  const body = step(dirs, run, "worktree", { key: "US-1" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", path: wt, branch: "story/US-1", created: false });
  assert.deepEqual([git(wt, "rev-parse", "HEAD").trim(), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), readFileSync(join(wt, "half.rs"), "utf8")], [work, "story/US-1", "half.rs\n"]);
});

test("a worktree an implementer left clean, with another branch or a detached HEAD checked out, goes back on story/<key>: the body names what it had and its commits not on story/<key>, oldest first, for a cherry-pick, and those commits stay where they are", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const { run } = start(dirs);
  const side = storyWork(dirs, run, "US-1");
  const onStory = sha(dirs, "refs/heads/story/US-1");
  git(side, "checkout", "-q", "-b", "side");
  const first = commit(side, "first.rs");
  const second = commit(side, "second.rs");
  const loose = storyWork(dirs, run, "US-2");
  git(loose, "checkout", "-q", "--detach");
  const detached = commit(loose, "loose.rs");

  const named = step(dirs, run, "worktree", { key: "US-1" });
  const headless = step(dirs, run, "worktree", { key: "US-2" });

  const one = (rev, file) => `${rev.slice(0, 7)} feat: ${file}`;
  assert.deepEqual(pick(named), { ok: true, key: "US-1", path: side, branch: "story/US-1", created: false, head: "side", commits: [one(first, "first.rs"), one(second, "second.rs")], back: true, reason: `${side} had the branch side checked out, not story/US-1, which is what lands: the workflow checked story/US-1 out again. Bring over its 2 commits not on story/US-1 with git cherry-pick, oldest first: ${one(first, "first.rs")}; ${one(second, "second.rs")}.` });
  assert.deepEqual(pick(headless), { ok: true, key: "US-2", path: loose, branch: "story/US-2", created: false, head: null, commits: [one(detached, "loose.rs")], back: true, reason: `${loose} had a detached HEAD at ${detached.slice(0, 7)} checked out, not story/US-2, which is what lands: the workflow checked story/US-2 out again. Bring over its 1 commit not on story/US-2 with git cherry-pick, oldest first: ${one(detached, "loose.rs")}.` });
  assert.deepEqual([git(side, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(side, "rev-parse", "HEAD").trim(), git(side, "status", "--porcelain"), sha(dirs, "refs/heads/side")], ["story/US-1", onStory, "", second]);
  assert.deepEqual([git(loose, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(loose, "cat-file", "-t", detached).trim()], ["story/US-2", "commit"]);
});

test("a worktree an implementer left with another branch or a detached HEAD checked out and work in progress stays as it is, back false, and the body says what to do first: commit or stash uncommitted changes, finish or abort a rebase in progress, remove an index.lock", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);
  const { run } = start(dirs);
  const changed = storyWork(dirs, run, "US-1");
  git(changed, "checkout", "-q", "-b", "side");
  const onSide = commit(changed, "side.rs");
  writeFileSync(join(changed, "README"), "half done\n");
  // US-2's implementer stopped in a rebase of its own, on a conflict, on a detached HEAD.
  const stuck = step(dirs, run, "worktree", { key: "US-2" }).path;
  writeFileSync(join(stuck, "README"), "the story's README\n");
  git(stuck, "commit", "-q", "-am", "feat: README");
  landOn(dirs, "deliver/board", "README");
  assert.throws(() => git(stuck, "rebase", "deliver/board"));
  const rebasing = git(stuck, "rev-parse", "HEAD").trim();
  const locked = storyWork(dirs, run, "US-3");
  git(locked, "checkout", "-q", "-b", "other");
  const lock = join(dirs.repo, ".git", "worktrees", "US-3", "index.lock");
  writeFileSync(lock, "");

  const [left, mid, stopped] = ["US-1", "US-2", "US-3"].map((key) => step(dirs, run, "worktree", { key }));

  const one = `${onSide.slice(0, 7)} feat: side.rs`;
  assert.deepEqual(pick(left), { ok: true, key: "US-1", path: changed, branch: "story/US-1", created: false, head: "side", commits: [one], back: false, reason: `${changed} has the branch side checked out, not story/US-1, which is what lands, with uncommitted changes in 1 file (README), so the workflow left it as it is: commit or stash the changes, check story/US-1 out, bring over the 1 commit not on story/US-1 with git cherry-pick, oldest first (${one}), and commit the story's work there.` });
  assert.deepEqual(pick(mid), { ok: true, key: "US-2", path: stuck, branch: "story/US-2", created: false, head: null, commits: [], back: false, reason: `${stuck} has a detached HEAD at ${rebasing.slice(0, 7)} checked out, not story/US-2, which is what lands, with a rebase of story/US-2 in progress, so the workflow left it as it is: finish the rebase or abort it, check story/US-2 out, and commit the story's work there.` });
  assert.deepEqual(pick(stopped), { ok: true, key: "US-3", path: locked, branch: "story/US-3", created: false, head: "other", commits: [], back: false, reason: `${locked} has the branch other checked out, not story/US-3, which is what lands, with ${lock}, which a git command that was stopped left, so the workflow left it as it is: remove the index.lock once no git command runs there, check story/US-3 out, and commit the story's work there.` });
  assert.deepEqual([git(changed, "rev-parse", "--abbrev-ref", "HEAD").trim(), readFileSync(join(changed, "README"), "utf8"), existsSync(join(dirs.repo, ".git", "worktrees", "US-2", "rebase-merge")), existsSync(lock)], ["side", "half done\n", true, true]);
});

test("a worktree step cut off while it put a worktree back on story/<key> is carried on by its repeat, which answers as the first try found it: the branch it had, and its commits to cherry-pick", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  git(wt, "checkout", "-q", "-b", "side");
  const work = commit(wt, "side.rs");
  // The switch back runs post-checkout once story/US-1 is checked out.
  const held = hook(dirs, "post-checkout", "sleep 30");

  await cutOff(t, dirs, run, "worktree", { opId: "wt-back", key: "US-1" }, () => git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim() === "story/US-1");
  rmSync(held);
  const again = step(dirs, run, "worktree", { opId: "wt-back", key: "US-1" });

  assert.deepEqual([again.ok, again.created, again.head, again.commits, again.back], [true, false, "side", [`${work.slice(0, 7)} feat: side.rs`], true]);
  assert.match(again.reason, /^\S+ had the branch side checked out, not story\/US-1, which is what lands: the workflow checked story\/US-1 out again\. Bring over its 1 commit not on story\/US-1 with git cherry-pick/);
  assert.deepEqual([git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(wt, "status", "--porcelain")], ["story/US-1", ""]);
});

test("context says whether the clarity cache has a hash of the story, as clarified; a story version the old runner kept there is not one, since that runner recorded no test obligation", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);
  const { run } = start(dirs);
  writeFileSync(join(dirs.repo, ".omni", "clarity.json"), JSON.stringify({ "US-3": 7 }));

  step(dirs, run, "clarity-record", { key: "US-1" });

  assert.deepEqual(["US-1", "US-2", "US-3"].map((key) => step(dirs, run, "context", { key }).clarified), [true, false, false]);
});

test("rework moves a story sent back to the backlog and claims it for implementing, in one call; a rework cut off after its move is carried on by its repeat, which only claims", async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const { run } = start(dirs);
  dirs.edit((s) => {
    for (const held of s.stories) Object.assign(held, { status: "review", holder: { run, stage: "review" } });
  });

  const body = step(dirs, run, "rework", { key: "US-1" });
  await cutOff(t, dirs, run, "rework", { opId: "rework-cut", key: "US-2" }, () => dirs.story("US-2").status === "backlog", { FAKE_OMNI_HANG_AFTER: "story move US-2" });
  const again = step(dirs, run, "rework", { opId: "rework-cut", key: "US-2" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", status: "implementing", blocked: false, holder: run });
  assert.deepEqual([again.ok, again.status, again.holder], [true, "implementing", run]);
  assert.deepEqual(dirs.writes(), ["move US-1", "claim US-1", "move US-2", "claim US-2"]);
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

test("checks-set merges checks into the run's run.json and gives all six back, stack_reset among them; an unknown check, or a command that is not a string, is refused", (t) => {
  const dirs = scratch(t, [], [], { checks: { fast: "make fast", stack_up: "make up", stack: "make stack" } });
  const { run } = start(dirs);

  const body = step(dirs, run, "checks-set", { checks: { seed: "make seed NAME=x", lint: "make lint", fast: "make quick", stack_reset: "make down" } });
  const unknown = step(dirs, run, "checks-set", { checks: { stackup: "make up" } });
  const odd = step(dirs, run, "checks-set", { checks: { seed: 3 } });

  const checks = { fast: "make quick", stack_up: "make up", stack: "make stack", stack_reset: "make down", seed: "make seed NAME=x", lint: "make lint" };
  assert.deepEqual(body.checks, checks);
  assert.deepEqual(JSON.parse(readFileSync(runFile(dirs, run, "run.json"), "utf8")).checks, checks);
  assert.deepEqual([unknown.ok, unknown.error], [false, "no check stackup; the checks are fast, stack_up, stack, stack_reset, seed, lint"]);
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

test("job-start runs the command in a process group of its own, its output going to the job's log; job-wait answers done: false at maxSeconds while it runs, then the result once it ends, never its output, which the log holds", (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  // The job runs past its 2 s until the test lets it end, once the first wait
  // has answered (or for 10 s more), so that wait answers while it runs.
  const go = join(dirs.root, "go");

  const started = step(dirs, run, "job-start", { name: "fast", key: "US-1", cmd: `echo "leader $$ group $(ps -o pgid= -p $$)"; sleep 2; [ -e "${go}" ] || sleep 10; echo done`, cwd: lane });
  const begun = Date.now();
  const waiting = step(dirs, run, "job-wait", { id: started.id, maxSeconds: 0.5 });
  const waited = Date.now() - begun;
  writeFileSync(go, "");
  const ended = step(dirs, run, "job-wait", { id: started.id, maxSeconds: 30 });
  const failing = step(dirs, run, "job-start", { name: "stack", key: "US-1", cmd: "echo the stack is down; exit 3", cwd: lane });
  const failed = step(dirs, run, "job-wait", { id: failing.id, maxSeconds: 30 });

  assert.deepEqual(pick(started), { ok: true, id: "US-1-fast-1", log: runFile(dirs, run, join("jobs", "US-1-fast-1.log")) });
  assert.deepEqual(pick(waiting), { ok: true, id: "US-1-fast-1", done: false });
  assert.ok(waited >= 500, `${waited} ms`);
  assert.deepEqual(Object.keys(ended.result).sort(), ["code", "durationMs", "ended", "ok", "signal", "started", "timedOut"]);
  assert.deepEqual([ended.done, ended.result.ok, ended.result.code, ended.result.signal, ended.result.timedOut, ended.tail], [true, true, 0, null, false, undefined]);
  assert.ok(ended.result.durationMs >= 2000);
  const [, leader, group] = readFileSync(started.log, "utf8").match(/^leader (\d+) group\s+(\d+)\ndone\n$/);
  assert.equal(leader, group, "the command leads a group of its own");
  assert.deepEqual([failed.done, failed.result.ok, failed.result.code, failed.tail], [true, false, 3, undefined]);
  assert.equal(readFileSync(failing.log, "utf8"), "the stack is down\n");
});

test("a job's command runs with OMNI_RUN set to its run, and OMNI_STORY to its story's key, unset for a job without one, so that a project's check can tell runs and stories apart", (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  // A story the courier's own shell names is not the job's.
  const shell = { OMNI_STORY: "US-9", OMNI_RUN: "run-elsewhere" };

  const keyed = step(dirs, run, "job-start", { name: "seed", key: "US-1", cmd: 'echo "story=$OMNI_STORY run=$OMNI_RUN"', cwd: lane }, shell);
  const bare = step(dirs, run, "job-start", { name: "fast", cmd: 'echo "story=${OMNI_STORY-unset} run=$OMNI_RUN"', cwd: lane }, shell);

  for (const { id } of [keyed, bare]) step(dirs, run, "job-wait", { id, maxSeconds: 30 });
  assert.deepEqual([keyed, bare].map(({ log }) => readFileSync(log, "utf8")), [`story=US-1 run=${run}\n`, `story=unset run=${run}\n`]);
});

test("a job past its limit gets SIGTERM to its whole group, and its result says it timed out; its log ends naming the job, not its command", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const pidFile = join(dirs.root, "child.pid");

  const { id, log } = step(dirs, run, "job-start", { name: "stack", cmd: `sleep 30 & echo $! > ${pidFile}; wait`, cwd: lane, limitMinutes: 1 / 60 });
  const body = step(dirs, run, "job-wait", { id, maxSeconds: 30 });

  assert.deepEqual([body.done, body.result.ok, body.result.timedOut, body.result.signal], [true, false, true, "SIGTERM"]);
  assert.ok(body.result.durationMs >= 1000 && body.result.durationMs < 8000, `${body.result.durationMs} ms`);
  assert.ok(await eventually(() => gone(Number(readFileSync(pidFile, "utf8")))), "the command's own child was stopped with it");
  const text = readFileSync(log, "utf8");
  assert.ok(text.endsWith(`\nTimed out: the job ${id} ran past its limit of 1s and was stopped.\n`), text);
  assert.doesNotMatch(text, /sleep 30/);
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

test("a job's output reaches its log byte for byte, so a character split across two writes survives; a finding that names a job that failed quotes the log's last 2 KB from its first whole character", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run, lane } = start(dirs);
  // é (C3 A9) split across two writes, then a thousand three-byte 日 (E6 97 A5).
  const cmd = "printf '\\303'; sleep 0.3; printf '\\251 caf\\303\\251\\n'; i=0; while [ $i -lt 1000 ]; do printf '\\346\\227\\245'; i=$((i+1)); done; exit 1";

  const { id, log } = step(dirs, run, "job-start", { name: "fast", cmd, cwd: lane });
  const body = step(dirs, run, "job-wait", { id, maxSeconds: 30 });
  step(dirs, run, "finding", { key: "US-1", stage: "testing", findings: [{ text: "The fast check failed.", files: [], job: id }] });

  assert.equal(readFileSync(log, "utf8"), `é café\n${"日".repeat(1000)}`);
  assert.equal(body.tail, undefined);
  // 3009 bytes: the last 2048 begin inside a 日, whose two remaining bytes are dropped.
  assert.equal(dirs.story("US-1").findings[0].text, `[attempt 1] The fast check failed.\n\nThe end of its output:\n\`\`\`\n${"日".repeat(682)}\n\`\`\``);
});

test("a finding or a thread that names a job of the run gets the end of the job's log after its text, which the helper reads, so that no courier carries the output; one that names no job of the run is refused and writes nothing", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run, lane } = start(dirs);
  const { id } = step(dirs, run, "job-start", { name: "stack", cmd: "echo booting; echo FAILED: db is down; exit 1", cwd: lane });
  step(dirs, run, "job-wait", { id, maxSeconds: 30 });
  const block = "The end of its output:\n```\nbooting\nFAILED: db is down\n```";

  const finding = step(dirs, run, "finding", { key: "US-1", stage: "testing", findings: [{ text: "The stack check failed.", files: [], job: id }, { text: "Another", files: [] }] });
  const opened = step(dirs, run, "thread-open", { title: "Run: the lane stopped", body: "The lane stopped.\n", job: id });
  const unknown = [
    step(dirs, run, "finding", { key: "US-1", stage: "testing", findings: [{ text: "x", files: [], job: "US-1-nope-1" }] }),
    step(dirs, run, "thread-open", { title: "Run: x", body: "x\n", job: "../escape" }),
  ];

  assert.equal(finding.ok, true);
  assert.deepEqual(dirs.story("US-1").findings.map((f) => f.text), [`[attempt 1] The stack check failed.\n\n${block}`, "[attempt 1] Another"]);
  assert.equal(opened.ok, true);
  assert.deepEqual(questions(dirs).find((th) => th.id === opened.thread).posts.map((p) => p.body), [`The lane stopped.\n\n${block}\n`]);
  assert.deepEqual(unknown.map((b) => [b.ok, b.retry, b.error]), [[false, false, `no job US-1-nope-1 in run ${run}`], [false, false, `no job ../escape in run ${run}`]]);
  assert.equal(dirs.story("US-1").findings.length, 2);
  assert.equal(questions(dirs).length, 1);
});

test("job-kill stops a running job's group, and gives its result, its log saying so; a second kill finds it ended", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const { id, log } = step(dirs, run, "job-start", { name: "stack", cmd: "sleep 30", cwd: lane });
  const { pgid } = jobOf(dirs, run, id);

  const body = step(dirs, run, "job-kill", { id });
  const again = step(dirs, run, "job-kill", { id });

  assert.deepEqual([body.killed, body.done, body.result.ok, body.result.timedOut, body.result.signal], [true, true, false, false, "SIGTERM"]);
  assert.equal(body.tail, undefined);
  assert.ok(readFileSync(log, "utf8").endsWith(`\nStopped: the job ${id} was stopped before it ended.\n`));
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

test("job-wait answers a job whose supervisor is gone without its result as lost, and stops its command, which the group's leader still runs", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const { id } = step(dirs, run, "job-start", { name: "stack", cmd: "sleep 30", cwd: lane });
  const job = jobOf(dirs, run, id);
  process.kill(job.supervisor, "SIGKILL");
  await eventually(() => gone(job.supervisor));

  const body = step(dirs, run, "job-wait", { id, maxSeconds: 5 });

  assert.deepEqual([body.done, body.result.ok, body.result.error], [true, false, `the supervisor of the job ${id} ended without its result`]);
  assert.ok(await eventually(() => gone(-job.pgid)), "its command was stopped");
});

// A job whose supervisor was killed, its command still running.
async function orphan(dirs, run, cwd, cmd = "sleep 30") {
  const { id } = step(dirs, run, "job-start", { name: "stack", cmd, cwd });
  const job = jobOf(dirs, run, id);
  process.kill(job.supervisor, "SIGKILL");
  await eventually(() => gone(job.supervisor));
  return { id, job };
}

test("job-kill stops a job whose supervisor died: its group, whose leader still runs the job's command, gets SIGTERM, and the job is lost", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const { id, job } = await orphan(dirs, run, lane);

  const body = step(dirs, run, "job-kill", { id });

  assert.deepEqual([body.killed, body.done, body.result.ok, body.result.error], [true, true, false, `the supervisor of the job ${id} ended without its result`]);
  assert.ok(await eventually(() => gone(-job.pgid)), "its command was stopped");
});

test("start stops the job of an ended run whose supervisor died, and records it lost", async (t) => {
  const dirs = scratch(t);
  const first = start(dirs);
  // A command a shell runs with exec leads the group itself: the leader is sleep, not sh.
  const { id, job } = await orphan(dirs, first.run, first.lane, "exec sleep 30");
  writeLock(dirs, { ...readLock(dirs), pid: deadPid() });

  const next = start(dirs, { op: "start-2" });

  assert.ok(next.log.includes(`Stopped the job ${id} of run ${first.run}, which had ended`), next.log.join("\n"));
  assert.ok(await eventually(() => gone(-job.pgid)), "its command was stopped");
  assert.equal(JSON.parse(readFileSync(runFile(dirs, first.run, join("jobs", `${id}.json`)), "utf8")).error, `the supervisor of the job ${id} ended without its result`);
});

test("pids of a job that other processes took are never signalled: a supervisor's pid that runs something else, and a group whose leader runs another command; the job is lost all the same", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const { id, job } = await orphan(dirs, run, lane);
  process.kill(-job.pgid, "SIGKILL");
  await eventually(() => gone(-job.pgid));
  // The job started a minute ago, and processes started since have its pids:
  // one of them leads a group of its own.
  const other = spawn("sleep", ["61"], { stdio: "ignore" });
  const leader = spawn("sleep", ["62"], { stdio: "ignore", detached: true });
  t.after(() => { other.kill("SIGKILL"); try { process.kill(-leader.pid, "SIGKILL"); } catch {} });
  writeFileSync(runFile(dirs, run, join("jobs", `${id}.job.json`)), JSON.stringify({ ...job, started: ago(60000), supervisor: other.pid, pgid: leader.pid }));

  const body = step(dirs, run, "job-kill", { id }, { OMNI_DELIVER_KILL_GRACE_MS: "500" });
  await pause(500);

  assert.deepEqual([body.killed, body.done, body.result.error], [false, true, `the supervisor of the job ${id} ended without its result`]);
  assert.deepEqual([other.signalCode, other.exitCode, leader.signalCode, leader.exitCode], [null, null, null, null]);
});

test("a wait is kept to 0 to 480 seconds, so that it fits a courier's Bash call, and is 480 when the payload gives no number", () => {
  assert.deepEqual([-5, 0.5, 480, 1e9, undefined, "20"].map(waitSeconds), [0, 0.5, 480, 480, 480, 480]);
});

test("a job-start cut off after it made the job is carried on by its repeat: the same job, run once", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  // The job's supervisor waits for the job file's lock, which holds the step until it is cut off.
  const dir = runFile(dirs, run, "jobs");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "US-1-fast-1.job.json.lock"), `${liveProcess(t)} 1`);
  const obj = { opId: "job-cut", name: "fast", key: "US-1", cmd: "echo ran", cwd: lane };

  await cutOff(t, dirs, run, "job-start", obj, () => existsSync(join(dir, "US-1-fast-1.job.json")));
  rmSync(join(dir, "US-1-fast-1.job.json.lock"));
  const again = step(dirs, run, "job-start", obj);
  const waited = step(dirs, run, "job-wait", { id: again.id, maxSeconds: 30 });

  assert.deepEqual([again.ok, again.id], [true, "US-1-fast-1"]);
  assert.deepEqual(readdirSync(dir).filter((n) => n.endsWith(".job.json")), ["US-1-fast-1.job.json"]);
  assert.deepEqual([waited.done, waited.result.ok, readFileSync(again.log, "utf8")], [true, true, "ran\n"]);
});

// Loaded into a step with --import, it makes the file TRIED when the step
// tries to create the lock file TRY_LOCK, as withLock does to take a lock.
const TRY_HOOK = `import fs from "node:fs";
const write = fs.writeFileSync;
fs.writeFileSync = (file, ...rest) => {
  if (String(file) === process.env.TRY_LOCK) write(process.env.TRIED, "");
  return write(file, ...rest);
};
`;

test("a job-start records its job's id before it writes the job file: cut off between the two, it leaves no job file, and its repeat makes the job once", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const obj = { opId: "job-cut", name: "fast", key: "US-1", cmd: "echo ran", cwd: lane };
  // An earlier try left its pending marker, and the record's lock, held by a
  // live process, holds this try where it records the job's id.
  const applied = runFile(dirs, run, "applied.json");
  writeFileSync(applied, JSON.stringify({ ...appliedOf(dirs, run), "job-start": { "job-cut": { pending: true, received: fnv1a(JSON.stringify(obj)) } } }));
  writeFileSync(`${applied}.lock`, `${liveProcess(t)} 1`);
  const dir = runFile(dirs, run, "jobs");
  const tryHook = join(dirs.root, "try.mjs");
  writeFileSync(tryHook, TRY_HOOK);
  const tried = join(dirs.root, "tried");

  // Cut off once it tries to take the record's lock, to record the id: with
  // the marker there, that is its first try at that lock.
  await cutOff(t, dirs, run, "job-start", obj, () => existsSync(tried), { NODE_OPTIONS: `--import=${pathToFileURL(tryHook).href}`, TRY_LOCK: `${applied}.lock`, TRIED: tried });
  const cut = readdirSync(dir).filter((n) => n.endsWith(".job.json"));
  rmSync(`${applied}.lock`);
  const again = step(dirs, run, "job-start", obj);
  const waited = step(dirs, run, "job-wait", { id: again.id, maxSeconds: 30 });

  assert.deepEqual(cut, []);
  assert.deepEqual(readdirSync(dir).filter((n) => n.endsWith(".job.json")), [`${again.id}.job.json`]);
  assert.deepEqual([waited.done, waited.result.ok, readFileSync(again.log, "utf8")], [true, true, "ran\n"]);
});

test("job-kill finds the job of a job-start whose answer never came back by that start's opId: a job its supervisor has not started yet is lost, and the supervisor never starts it; a start that made no job has nothing to stop", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  // The job's supervisor waits for the job file's lock, which holds the step until it is cut off.
  const dir = runFile(dirs, run, "jobs");
  mkdirSync(dir, { recursive: true });
  const lock = join(dir, "US-1-fast-1.job.json.lock");
  writeFileSync(lock, `${liveProcess(t)} 1`);
  const ran = join(dirs.root, "ran");
  const obj = { opId: "job-cut", name: "fast", key: "US-1", cmd: `touch ${ran}`, cwd: lane };

  // Cut off once the supervisor it spawned runs, as ps shows it.
  await cutOff(t, dirs, run, "job-start", obj, () => supervisorOf(join(dir, "US-1-fast-1.job.json")));
  // job-kill takes that lock too: the supervisor, stopped where it waits, takes it only after job-kill.
  const supervisor = supervisorOf(join(dir, "US-1-fast-1.job.json"));
  process.kill(supervisor, "SIGSTOP");
  t.after(() => { try { process.kill(supervisor, "SIGKILL"); } catch {} });
  rmSync(lock);
  const body = step(dirs, run, "job-kill", { start: "job-cut" });
  process.kill(supervisor, "SIGCONT");
  assert.ok(await eventually(() => gone(supervisor)), "the supervisor left");
  const none = step(dirs, run, "job-kill", { start: "job-never" });

  assert.deepEqual([body.ok, body.id, body.killed, body.done, body.result?.ok, body.result?.error], [true, "US-1-fast-1", false, true, false, "the supervisor of the job US-1-fast-1 ended without its result"]);
  assert.equal(existsSync(ran), false, "the job never started");
  assert.deepEqual(pick(none), { ok: true, id: null, killed: false, done: false });
});

// Loaded into a supervisor with --import, it holds the supervisor inside the
// job file's lock, past its look for a result, where it opens the job's log,
// while a job-kill it starts there runs: until that job-kill has answered, or
// has tried to take the lock, which this file, loaded into it too, marks.
const RACE_HOOK = `import fs from "node:fs";
import { spawn } from "node:child_process";
const race = JSON.parse(process.env.RACE);
const nap = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
if (process.argv.includes("__supervise")) {
  const open = fs.openSync;
  fs.openSync = (...a) => {
    if (a[0] === race.log && a[1] === "a" && !fs.existsSync(race.out)) {
      const out = open(race.out, "w");
      spawn(process.execPath, ["--import", race.hook, ...race.kill], { cwd: race.cwd, stdio: ["ignore", out, "ignore"] });
      for (const until = Date.now() + 20000; Date.now() < until && !fs.existsSync(race.tried) && !fs.readFileSync(race.out, "utf8").endsWith("\\n"); ) nap(20);
    }
    return open(...a);
  };
} else {
  const write = fs.writeFileSync;
  fs.writeFileSync = (file, ...rest) => {
    if (String(file).endsWith(".job.json.lock")) write(race.tried, "");
    return write(file, ...rest);
  };
}
`;

test("job-kill never loses a job its supervisor is starting: the supervisor looked for a result under the job file's lock, and job-kill, which waits for that lock, finds the job started and stops it, and the result it gives stays", async (t) => {
  const dirs = scratch(t);
  const { run, lane } = start(dirs);
  const dir = runFile(dirs, run, "jobs");
  mkdirSync(dir, { recursive: true });
  // A job-start whose answer never came back made the job, and spawned its supervisor.
  const jobFile = join(dir, "US-1-fast-1.job.json");
  writeFileSync(jobFile, JSON.stringify({ id: "US-1-fast-1", run, name: "fast", key: "US-1", cmd: "sleep 5", cwd: lane, limitMs: 60000 }));
  writeFileSync(join(dir, "US-1-fast-1.log"), "");
  writeFileSync(runFile(dirs, run, "applied.json"), JSON.stringify({ ...appliedOf(dirs, run), "job-start": { "js-1": { pending: true, received: "x", id: "US-1-fast-1" } } }));
  const hook = join(dirs.root, "race.mjs");
  writeFileSync(hook, RACE_HOOK);
  const race = { log: join(dir, "US-1-fast-1.log"), out: join(dirs.root, "kill.out"), tried: join(dirs.root, "kill.tried"), hook: pathToFileURL(hook).href, kill: [helper, ...stepArgv(dirs, run, "job-kill", { opId: "kill-1", start: "js-1" })], cwd: dirs.repo };
  const supervisor = spawn(process.execPath, ["--import", race.hook, helper, "__supervise", jobFile], { cwd: dirs.repo, env: { ...dirs.env, RACE: JSON.stringify(race) }, detached: true, stdio: "ignore" });
  let exited = false;
  supervisor.on("exit", () => { exited = true; });
  t.after(() => { try { process.kill(supervisor.pid, "SIGKILL"); } catch {} });

  assert.ok(await eventually(() => existsSync(race.out) && readFileSync(race.out, "utf8").endsWith("\n"), 30000), "job-kill answered");
  assert.ok(await eventually(() => exited, 15000), "the supervisor ended");
  const { body } = JSON.parse(readFileSync(race.out, "utf8"));
  const result = JSON.parse(readFileSync(join(dir, "US-1-fast-1.json"), "utf8"));

  assert.deepEqual([body.ok, body.id, body.killed, body.done, body.result.ok, body.result.signal], [true, "US-1-fast-1", true, true, false, "SIGTERM"]);
  assert.ok(readFileSync(race.log, "utf8").endsWith("\nStopped: the job US-1-fast-1 was stopped before it ended.\n"));
  assert.deepEqual(result, body.result);
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

  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "conflict", files: ["README"], count: 1 });
  assert.deepEqual([git(wt, "rev-parse", "HEAD").trim(), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(wt, "status", "--porcelain")], [before, "story/US-1", ""]);
  assert.equal(git(lane, "rev-parse", "HEAD").trim(), laneHead);
});

test("lane-prepare gives a conflict in more than 20 files as the first 20 and the count of them all, since outputs stay small", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const names = Array.from({ length: 25 }, (_, i) => `f${String(i).padStart(2, "0")}.rs`);
  const both = (cwd, text) => {
    for (const name of names) writeFileSync(join(cwd, name), text);
    git(cwd, "add", ...names);
    git(cwd, "commit", "-q", "-m", `feat: ${text.trim()}`);
  };
  both(step(dirs, run, "worktree", { key: "US-1" }).path, "the story's\n");
  const other = join(dirs.root, "other");
  git(dirs.repo, "worktree", "add", "-q", other, "deliver/board");
  both(other, "deliver's\n");
  git(dirs.repo, "worktree", "remove", other);

  const body = step(dirs, run, "lane-prepare", { key: "US-1" });

  assert.deepEqual([body.outcome, body.files, body.count], ["conflict", names.slice(0, 20), 25]);
});

test("a rebase past its limit is infra, not a conflict: lane-prepare stops it, undoes it, and says so", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  landOn(dirs, "deliver/board", "landed.rs");
  const before = git(wt, "rev-parse", "HEAD").trim();
  // The hook holds the rebase for as long as the test's folders are there, 30 s
  // at most, and says so only when it ends at those 30 s: git leaves it
  // running, and it writes nothing that the test's cleanup could race.
  const hookEnded = join(dirs.root, "hook-ended");
  hook(dirs, "pre-rebase", `i=0; while [ -d "${dirs.root}" ] && [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done; [ $i -lt 300 ] || touch "${hookEnded}"`);

  const body = step(dirs, run, "lane-prepare", { key: "US-1" }, { OMNI_DELIVER_REBASE_MS: "1000" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "infra", reason: "the rebase of story/US-1 onto deliver/board ran past its limit of 1s" });
  assert.equal(existsSync(hookEnded), false, "stopped at its limit, not at the hook's end");
  assert.deepEqual([git(wt, "rev-parse", "HEAD").trim(), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim()], [before, "story/US-1"]);
});

test("lane-prepare refuses a story worktree on another branch or on a detached HEAD as wrong_branch, naming what is checked out and the commits that are not on story/<key>, which would not land; nothing is rebased, and the lane stays", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const { run, lane } = start(dirs);
  const own = step(dirs, run, "worktree", { key: "US-1" }).path;
  git(own, "checkout", "-q", "-b", "fix-badge");
  const onBranch = commit(own, "badge.rs");
  const detached = storyWork(dirs, run, "US-2");
  const story2 = sha(dirs, "refs/heads/story/US-2");
  git(detached, "checkout", "-q", "--detach");
  const loose = commit(detached, "fix.rs");
  landOn(dirs, "deliver/board", "landed.rs");
  const laneHead = git(lane, "rev-parse", "HEAD").trim();

  const named = step(dirs, run, "lane-prepare", { key: "US-1" });
  const headless = step(dirs, run, "lane-prepare", { key: "US-2" });

  const one = (rev, file) => `${rev.slice(0, 7)} feat: ${file}`;
  assert.deepEqual(pick(named), { ok: true, key: "US-1", outcome: "wrong_branch", head: "fix-badge", commits: [one(onBranch, "badge.rs")], reason: `${own} has the branch fix-badge checked out, not story/US-1: only story/US-1 lands, so its 1 commit not on story/US-1 would be lost (${one(onBranch, "badge.rs")}). Commit on story/US-1` });
  assert.deepEqual(pick(headless), { ok: true, key: "US-2", outcome: "wrong_branch", head: null, commits: [one(loose, "fix.rs")], reason: `${detached} has a detached HEAD at ${loose.slice(0, 7)}, not story/US-2: only story/US-2 lands, so its 1 commit not on story/US-2 would be lost (${one(loose, "fix.rs")}). Commit on story/US-2` });
  assert.deepEqual([git(own, "rev-parse", "HEAD").trim(), sha(dirs, "refs/heads/story/US-2"), git(lane, "rev-parse", "HEAD").trim()], [onBranch, story2, laneHead]);
});

test("lane-prepare gives dirty, with the files, for what the implementer left half done in the story's own worktree, which would not land, on whatever branch: uncommitted changes, a rebase in progress that is not its own, which stays, and an index.lock; a rebase that fails on anything but unmerged paths is infra, with git's message", (t) => {
  const dirs = scratch(t, ["US-1", "US-2", "US-3", "US-4", "US-5"].map((key) => story(key)));
  const { run } = start(dirs);
  const dirty = storyWork(dirs, run, "US-1");
  writeFileSync(join(dirty, "README"), "left uncommitted\n");
  writeFileSync(join(dirty, "notes.txt"), "untracked\n");
  // US-2's implementer left a rebase of its own stopped on a conflict.
  const stuck = storyWork(dirs, run, "US-2", "README");
  git(stuck, "checkout", "-q", "-b", "other", "HEAD~1");
  writeFileSync(join(stuck, "README"), "other\n");
  git(stuck, "commit", "-q", "-am", "other README");
  git(stuck, "checkout", "-q", "story/US-2");
  assert.throws(() => git(stuck, "rebase", "other"));
  // US-4's implementer was stopped in the middle of a git command.
  const locked = storyWork(dirs, run, "US-4");
  const lock = join(dirs.repo, ".git", "worktrees", "US-4", "index.lock");
  writeFileSync(lock, "");
  // US-5's implementer left another branch checked out, with a change it did not commit.
  const away = storyWork(dirs, run, "US-5");
  git(away, "checkout", "-q", "-b", "side");
  writeFileSync(join(away, "README"), "left on side\n");
  storyWork(dirs, run, "US-3");
  landOn(dirs, "deliver/board", "landed.rs");
  const refusing = () => hook(dirs, "pre-rebase", "exit 1");

  const changed = step(dirs, run, "lane-prepare", { key: "US-1" });
  const midRebase = step(dirs, run, "lane-prepare", { key: "US-2" });
  const stopped = step(dirs, run, "lane-prepare", { key: "US-4" });
  const elsewhere = step(dirs, run, "lane-prepare", { key: "US-5" });
  refusing();
  const hooked = step(dirs, run, "lane-prepare", { key: "US-3" });

  assert.deepEqual(pick(changed), { ok: true, key: "US-1", outcome: "dirty", files: ["README", "notes.txt"], count: 2, reason: `${dirty} has uncommitted changes in 2 files, which would not land: commit or discard them` });
  assert.deepEqual(pick(midRebase), { ok: true, key: "US-2", outcome: "dirty", files: ["README"], count: 1, reason: `a rebase of story/US-2 is in progress in ${stuck}: finish it or abort it, and commit the story's work on story/US-2` });
  assert.ok(existsSync(join(dirs.repo, ".git", "worktrees", "US-2", "rebase-merge")), "the rebase in progress stays");
  assert.deepEqual(pick(stopped), { ok: true, key: "US-4", outcome: "dirty", files: [], count: 0, reason: `${locked} has ${lock}, which a git command that was stopped left, and every git command there that writes fails on it: remove it once no git command runs there` });
  assert.deepEqual(pick(elsewhere), { ok: true, key: "US-5", outcome: "dirty", files: ["README"], count: 1, reason: `${away} has uncommitted changes in 1 file, which would not land: commit or discard them` });
  assert.deepEqual([existsSync(lock), git(away, "rev-parse", "--abbrev-ref", "HEAD").trim()], [true, "side"]);
  assert.equal(hooked.outcome, "infra");
  assert.match(hooked.reason, /^the rebase of story\/US-3 onto deliver\/board failed: .*pre-rebase hook refused to rebase/);
});

test("lane-prepare does nothing in a story folder that is not a worktree of its own: in one that lost its .git file, inside a repository stopped in a rebase, that rebase stays as it was", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  // The folder around the checkout is a repository of its own, stopped in a rebase on a conflict.
  const outer = (...argv) => git(dirs.root, ...argv);
  outer("init", "-q", "-b", "main");
  outer("config", "user.email", "test@example.com");
  outer("config", "user.name", "Test");
  writeFileSync(join(dirs.root, ".gitignore"), "app/\napp-stories/\nbin/\nomni-state.json*\n");
  writeFileSync(join(dirs.root, "notes.txt"), "base\n");
  outer("add", ".gitignore", "notes.txt");
  outer("commit", "-q", "-m", "base");
  outer("checkout", "-q", "-b", "side");
  writeFileSync(join(dirs.root, "notes.txt"), "side\n");
  outer("commit", "-q", "-am", "side");
  outer("checkout", "-q", "main");
  writeFileSync(join(dirs.root, "notes.txt"), "main\n");
  outer("commit", "-q", "-am", "main");
  assert.throws(() => outer("rebase", "side"));
  rmSync(join(wt, ".git"));

  const body = step(dirs, run, "lane-prepare", { key: "US-1" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "missing", reason: `${wt} is not a worktree of ${dirs.repo} of its own, so git does nothing in it` });
  assert.ok(existsSync(join(dirs.root, ".git", "rebase-merge")), "the outer rebase is still in progress");
});

test("a lane-prepare cut off in the middle of its rebase is carried on by its repeat, which undoes that rebase of story/<key> and does it again", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run, lane } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  const tip = landOn(dirs, "deliver/board", "landed.rs");
  // A rebase checks the base out, and its post-checkout hook holds it there.
  // The hook says so when it runs, after that checkout has let index.lock go;
  // the rebase-merge folder is there before the checkout, and a cut-off then
  // leaves index.lock behind.
  const checkedOut = join(dirs.root, "checked-out");
  const held = hook(dirs, "post-checkout", `[ -d "$(git rev-parse --git-path rebase-merge)" ] && touch "${checkedOut}" && sleep 30; exit 0`);
  const state = join(dirs.repo, ".git", "worktrees", "US-1", "rebase-merge");

  await cutOff(t, dirs, run, "lane-prepare", { opId: "prepare-cut", key: "US-1" }, () => existsSync(checkedOut));
  rmSync(held);
  const again = step(dirs, run, "lane-prepare", { opId: "prepare-cut", key: "US-1" });

  assert.deepEqual([again.outcome, again.base], ["ready", tip]);
  assert.deepEqual([existsSync(state), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(wt, "rev-parse", "HEAD^").trim(), git(lane, "rev-parse", "HEAD").trim()], [false, "story/US-1", tip, again.candidate]);
});

test("lane-prepare gives missing for a story worktree without its branch", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run } = start(dirs);
  git(dirs.repo, "worktree", "add", "-q", "--detach", join(dirs.root, "app-stories", "US-1"));

  const body = step(dirs, run, "lane-prepare", { key: "US-1" });

  assert.deepEqual(pick(body), { ok: true, key: "US-1", outcome: "missing", reason: "no branch story/US-1" });
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

  assert.deepEqual(pick(body), { ok: true, key: "US-1", landed: true, base, final: tested, files: 2, removed: true });
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

test("lane-land refuses a lane HEAD that does not descend from the base, or does not hold the story's branch, as the lane's state, inLane, and moves nothing", (t) => {
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

  assert.deepEqual([dropped.landed, dropped.reason, dropped.inLane], [false, `the lane's HEAD ${testsOnly.slice(0, 7)} does not hold story/US-1 at ${story1.slice(0, 7)}`, true]);
  assert.deepEqual([elsewhere.landed, elsewhere.reason, elsewhere.inLane], [false, `the lane's HEAD ${offBase.slice(0, 7)} does not descend from the base ${base.slice(0, 7)}`, true]);
  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), dirs.story("US-1").status], [tip, "testing"]);
});

test("lane-land refuses a lane worktree with a branch checked out, as the lane's state, inLane: a commit the test author made on deliver/<slug> there moved the branch without a swap, so the branch goes back to the base by a compare-and-swap and the lane is detached; with another branch checked out, deliver/<slug> stays where it is, moved by another writer as it may be; and a lane HEAD is not landed when the story has no branch", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" }), story("US-2", { status: "reviewed" })]);
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  git(lane, "checkout", "-q", "deliver/board");
  const moved = commit(lane, "badge.test");
  const where = () => [sha(dirs, "refs/heads/deliver/board"), git(lane, "rev-parse", "--abbrev-ref", "HEAD").trim()];

  const onBranch = step(dirs, run, "lane-land", { key: "US-1", base });
  const afterOnBranch = where();
  step(dirs, run, "lane-reset");
  git(lane, "checkout", "-q", "-b", "tests");
  const tests = commit(lane, "other.test");
  const elsewhere = landOn(dirs, "deliver/board", "elsewhere.rs");
  const onOther = step(dirs, run, "lane-land", { key: "US-1", base });
  const afterOnOther = [...where(), git(lane, "rev-parse", "HEAD").trim()];
  const noBranch = step(dirs, run, "lane-land", { key: "US-2", base });

  assert.deepEqual(pick(onBranch), { ok: true, key: "US-1", landed: false, inLane: true, reason: `the lane worktree had deliver/board checked out, not a detached HEAD: a commit there moves that branch without the compare-and-swap. deliver/board is back at the base ${base.slice(0, 7)}, from ${moved.slice(0, 7)}, and the lane is now detached` });
  assert.deepEqual(afterOnBranch, [base, "HEAD"]);
  assert.deepEqual(pick(onOther), { ok: true, key: "US-1", landed: false, inLane: true, reason: "the lane worktree had tests checked out, not a detached HEAD: a commit there moves that branch without the compare-and-swap. The lane is now detached" });
  assert.deepEqual(afterOnOther, [elsewhere, "HEAD", tests]);
  assert.deepEqual(pick(noBranch), { ok: true, key: "US-2", landed: false, reason: "there is no branch story/US-2 to land" });
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-2").status], ["testing", "reviewed"]);
});

test("lane-clean throws away what is uncommitted in the lane worktree and says what, keeping ignored files and its HEAD; lane-land refuses a lane with uncommitted changes, so a fix the test author left uncommitted never lands unchecked", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  writeFileSync(join(dirs.repo, ".gitignore"), ".omni/\ntarget/\n");
  git(dirs.repo, "commit", "-q", "-am", "ignore target");
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  // The test author fixed the story's code to pass its test, and committed only the test.
  writeFileSync(join(lane, "badge.rs"), "fixed\n");
  const tested = commit(lane, "badge.test");
  mkdirSync(join(lane, "target"));
  writeFileSync(join(lane, "target", "build.out"), "warm\n");

  const refused = step(dirs, run, "lane-land", { key: "US-1", base });
  const cleaned = step(dirs, run, "lane-clean");
  const landed = step(dirs, run, "lane-land", { key: "US-1", base });

  assert.deepEqual(pick(refused), { ok: true, key: "US-1", landed: false, inLane: true, reason: "the lane worktree has uncommitted changes in 1 file (badge.rs): what lands must be what was checked, so lane-clean it and check it again" });
  assert.deepEqual(pick(cleaned), { ok: true, discarded: ["badge.rs"], count: 1 });
  assert.deepEqual([readFileSync(join(lane, "badge.rs"), "utf8"), readFileSync(join(lane, "target", "build.out"), "utf8"), git(lane, "rev-parse", "HEAD").trim()], ["badge.rs\n", "warm\n", tested]);
  assert.deepEqual([landed.landed, landed.final, sha(dirs, "refs/heads/deliver/board")], [true, tested, tested]);
});

test("a lane-land cut off after its swap is carried on by its repeat from its record of that swap: the branch moved once, and the story is done", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", criteria: [{ text: "The badge is red", human_check: false }] })]);
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  const tested = commit(lane, "badge.test");
  const obj = { opId: "land-cut", key: "US-1", base };

  await cutOff(t, dirs, run, "lane-land", obj, () => (dirs.story("US-1").files_changed || []).length > 0, { FAKE_OMNI_HANG_AFTER: "story files US-1" });
  const marker = appliedOf(dirs, run, "lane-land")["land-cut"];
  const again = step(dirs, run, "lane-land", obj);

  assert.deepEqual([marker.pending, marker.swapped], [true, tested]);
  assert.deepEqual([again.ok, again.landed, again.final, again.files], [true, true, tested, 2]);
  assert.deepEqual([sha(dirs, "refs/heads/deliver/board"), dirs.story("US-1").status, dirs.story("US-1").criteria[0].ticked], [tested, "done", true]);
});

test("lane-land leaves the story's folder alone when it is not its worktree on story/<key>, and says so: removed false", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const { run, lane } = start(dirs);
  const wt = storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  const { base } = step(dirs, run, "lane-prepare", { key: "US-1" });
  commit(lane, "badge.test");
  git(wt, "checkout", "-q", "-b", "other");

  const body = step(dirs, run, "lane-land", { key: "US-1", base });

  assert.deepEqual([body.landed, body.removed, dirs.story("US-1").status], [true, false, "done"]);
  assert.deepEqual([existsSync(wt), git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim()], [true, "other"]);
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

test("lane-candidate puts the lane worktree back at the candidate lane-prepare gave, clean and detached, without what the test author committed; a candidate that is not a commit's full hash is refused", (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  const { candidate } = step(dirs, run, "lane-prepare", { key: "US-1" });
  commit(lane, "badge.test");
  writeFileSync(join(lane, "stray.txt"), "left behind\n");

  const body = step(dirs, run, "lane-candidate", { candidate });
  const odd = step(dirs, run, "lane-candidate", { candidate: "HEAD~1" });

  assert.deepEqual(pick(body), { ok: true, head: candidate });
  assert.deepEqual([git(lane, "rev-parse", "HEAD").trim(), git(lane, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(lane, "status", "--porcelain")], [candidate, "HEAD", ""]);
  assert.deepEqual([odd.ok, odd.error], [false, "the candidate is a commit's full hash, not HEAD~1"]);
});

// ---- finish -----------------------------------------------------------------------------

test("finish stops the run's jobs that still run, posts the report on the run's thread from a body file, with the blocked stories and their threads from the board, keeps it as report.md, and gives the lock up", async (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "reviewed", criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }] }),
    story("US-2", { blocked: true, links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }, questionLink("th-1")] }),
    story("US-3", { blocked: true }),
    story("US-4", { status: "cancelled", blocked: true, links: [questionLink("th-2")] }),
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
- US-3 Story US-3: thread (none linked)

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
  const time = String.raw`(?:<1s|\d+s)`;
  assert.match(timings, new RegExp(String.raw`^The run took ${time}\.\n\n\| Story \| Total \| implementing \| review \| testing \| rebase \| fast \| stack \|\n\|---\|---\|---\|---\|---\|---\|---\|---\|\n\| US-1 \| ${time} \| - \| - \| ${time} \| ${time} \| ${time} \| ${time} \|\n\nA time with \(n\) after it adds up n of them: claims of the stage, rebases, or runs of the job\.\n$`));
});

const noReport = { outcomes: [], council: { settled: 0, asked: 0 } };

test("a finish whose post failed keeps the report as report.md, gives the lock up, and says it may be tried again; its repeat with the same opId goes on without the lock and posts the report once", (t) => {
  const dirs = scratch(t);
  const { run, thread: runThread } = start(dirs);
  const obj = { opId: "finish-1", report: noReport };
  const threads = dirs.state().threads;
  // omni cannot find the Run thread for a while.
  dirs.edit((s) => { s.threads = []; });

  const failed = step(dirs, run, "finish", obj);
  const lockAfter = existsSync(dirs.lock);
  dirs.edit((s) => { s.threads = threads; });
  const again = step(dirs, run, "finish", obj);

  assert.deepEqual([failed.ok, failed.posted, failed.retry, failed.report, lockAfter], [false, false, true, runFile(dirs, run, "report.md"), false]);
  assert.match(failed.error, /^the report could not be posted: /);
  assert.deepEqual(pick(again), { ok: true, thread: runThread, posted: true, report: failed.report, stopped: [] });
  assert.deepEqual(dirs.state().threads.find((th) => th.id === runThread).posts.map((p) => [p.summary, p.body]), [[`Report of ${run}`, readFileSync(failed.report, "utf8")]]);
  assert.equal(existsSync(dirs.lock), false);
});

test("a finish cut off right after its post finds the report on the Run thread, and its repeat does not post it again", async (t) => {
  const dirs = scratch(t);
  const { run, thread: runThread } = start(dirs);
  const obj = { opId: "finish-cut", report: noReport };
  const posts = () => dirs.state().threads.find((th) => th.id === runThread).posts;

  await cutOff(t, dirs, run, "finish", obj, () => posts().length === 1, { FAKE_OMNI_HANG_AFTER: `post ${runThread}` });
  const again = step(dirs, run, "finish", obj);

  assert.deepEqual([again.ok, again.posted], [true, true]);
  assert.deepEqual(posts().map((p) => p.summary), [`Report of ${run}`]);
  assert.equal(existsSync(dirs.lock), false);
});

test("the report's timings sort the job columns by name, show a time under a second as <1s, say what a count means, open no stage for a claim the board refused, and time the rebase apart from the lane's checkout", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" }), story("US-2")]);
  const { run, lane } = start(dirs);
  storyWork(dirs, run, "US-1");
  step(dirs, run, "claim", { key: "US-1", stage: "testing" });
  // The lane's checkout takes 2 s, and the rebase none of it.
  hook(dirs, "post-checkout", '[ -d "$(git rev-parse --git-path rebase-merge)" ] || sleep 2');
  step(dirs, run, "lane-prepare", { key: "US-1" });
  // Their ids sort the other way: US-1-zeta-1 before US-2-alpha-1.
  for (const [key, name] of [["US-1", "zeta"], ["US-2", "alpha"]]) step(dirs, run, "job-wait", { id: step(dirs, run, "job-start", { name, key, cmd: "true", cwd: lane }).id, maxSeconds: 30 });
  const refused = step(dirs, run, "claim", { key: "US-1", stage: "review" });

  const body = step(dirs, run, "finish", { report: noReport });

  assert.equal(refused.ok, false);
  const timings = readFileSync(body.report, "utf8").split("## Timings\n")[1];
  const time = String.raw`(?:<1s|\d+s)`;
  assert.match(timings, new RegExp(String.raw`^The run took ${time}\.\n\n\| Story \| Total \| implementing \| review \| testing \| rebase \| alpha \| zeta \|\n\|---\|---\|---\|---\|---\|---\|---\|---\|\n\| US-1 \| ${time} \| - \| - \| [1-9]\d*s \| <1s \| - \| <1s \|\n\| US-2 \| ${time} \| - \| - \| - \| - \| <1s \| - \|\n\nA time with \(n\) after it adds up n of them: claims of the stage, rebases, or runs of the job\.\n$`), timings);
});

// ---- the size of what a courier carries back ----------------------------------------------

test("no output the script reads passes 2,000 characters, however long a story's text, criteria, decisions and findings, a thread's answer or a job's log: a courier copies every byte back", (t) => {
  const huge = (label, n) => `${label}: ${'a "quoted" line\\ with a backslash, and a new one\n'.repeat(n)}`;
  const many = (n, make) => Array.from({ length: n }, (_, i) => make(i));
  const files = many(30, (j) => `src/a-rather-long-folder-name/and-a-longer-file-name-${j}.rs`);
  const big = {
    title: huge("title", 10),
    text: huge("story", 2000),
    criteria: many(30, (i) => ({ text: huge(`criterion ${i}`, 40), human_check: i % 5 === 0 })),
    decisions: many(30, (i) => ({ question: huge(`question ${i}`, 40), choice: huge(`choice ${i}`, 40), by: i % 2 ? "agent" : "human" })),
    findings: [
      ...many(10, (i) => ({ stage: "review", text: `[attempt 1] ${huge(`answered ${i}`, 80)}`, files, resolved: true })),
      ...many(10, (i) => ({ stage: "review", text: `[attempt 1] ${huge(`open ${i}`, 80)}`, files, resolved: false })),
      ...many(10, (i) => ({ stage: "testing", text: `[attempt 2] ${huge(`check ${i}`, 80)}`, files: [], resolved: false })),
      ...many(5, (i) => ({ stage: "testing", text: huge(`conflict ${i}`, 80), files, resolved: false })),
    ],
    files_planned: files,
    links: [{ doc_id: "doc-spec", node_id: "n1", label: huge("label", 5) }],
  };
  const dirs = scratch(t, [
    story("US-1", big),
    story("US-2", { ...big, status: "implemented" }),
    story("US-3", { ...big, blocked: true, links: [questionLink("th-1")] }),
  ], [thread("th-1", huge("US-3: which?", 40), huge("the answer", 200))], { docs: { "doc-spec": `<p data-omni-id="n1">${huge("doc", 2000)}</p>` } });
  const sizes = [];
  // A step as a courier runs it, with the length of what it printed.
  const sized = (argv) => {
    const r = spawnSync(process.execPath, [helper, ...argv], { cwd: dirs.repo, env: dirs.env, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
    const body = JSON.parse(r.stdout).body;
    assert.equal(body.ok, true, `${argv[0]}: ${body.error}`);
    sizes.push([argv[0], r.stdout.length]);
    return body;
  };
  const go = (name, fields = {}) => sized(stepArgv(dirs, run, name, { opId: `${name}-${++ops}`, ...fields }));
  const { run, lane } = sized(startArgv());

  go("checks-set", { checks: { seed: "./seed", lint: "./lint" } });
  go("unblock-pass");
  go("clarity-pending");
  for (const key of ["US-1", "US-2", "US-3"]) go("context", { key });
  go("decision", { key: "US-1", question: huge("a new question", 100), choice: huge("a new choice", 100), by: "agent" });
  go("clarity-record", { key: "US-1" });
  go("list");
  go("claim", { key: "US-1", stage: "implementing" });
  const wt = go("worktree", { key: "US-1" }).path;
  commit(wt, "badge.rs");
  go("files", { key: "US-1" });
  go("move", { key: "US-1", to: "implemented" });
  go("claim", { key: "US-1", stage: "review" });
  go("move", { key: "US-1", to: "reviewed" });
  go("claim", { key: "US-1", stage: "testing" });
  const { base, candidate } = go("lane-prepare", { key: "US-1" });
  const { id } = go("job-start", { name: "fast", key: "US-1", cmd: `i=0; while [ $i -lt 2000 ]; do echo "error \\"$i\\" at C:\\\\path\\\\to\\\\file.rs, line $i"; i=$((i+1)); done; exit 1`, cwd: lane });
  go("job-wait", { id, maxSeconds: 30 });
  go("job-kill", { id });
  go("lane-clean");
  go("lane-base");
  go("lane-reset");
  go("lane-candidate", { candidate });
  go("lane-land", { key: "US-1", base });
  go("claim", { key: "US-2", stage: "review" });
  go("finding", { key: "US-2", stage: "testing", findings: [{ text: huge("a check failed", 100), files, job: id }, { text: huge("a defect", 100), files }], counts: true });
  go("rework", { key: "US-2" });
  go("release", { key: "US-2" });
  go("thread-open", { key: "US-2", title: huge("US-2: a question", 5), body: huge("the options", 200), job: id });
  go("exhaust", { key: "US-2" });
  go("unblock", { key: "US-2" });
  go("block", { key: "US-2" });
  go("sleep", { seconds: 0 });
  go("finish", { report: { outcomes: [{ key: "US-1", outcome: "done" }, { key: "US-2", outcome: "failed", detail: huge("an error", 100) }], council: { settled: 1, asked: 1 }, journeys: [huge("a journey", 100)], notes: [huge("a note", 100)] } });

  t.diagnostic(`the largest outputs: ${JSON.stringify([...sizes].sort((a, b) => b[1] - a[1]).slice(0, 3))}`);
  const over = sizes.filter(([, n]) => n > 2000);
  assert.deepEqual(over, [], `the largest: ${JSON.stringify([...sizes].sort((a, b) => b[1] - a[1]).slice(0, 5))}`);
  assert.ok(sizes.length >= 36, sizes.length);
});
