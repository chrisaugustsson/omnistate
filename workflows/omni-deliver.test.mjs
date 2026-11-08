// omni-deliver.js, the native workflow script, run under a fake of the
// Workflow runtime: agent, parallel, pipeline, phase and log with the
// runtime's semantics (parallel is a barrier, pipeline has none, and a thunk
// or a stage that throws gives null), with Date, Date.now and Math.random
// throwing as they do there. The fake agent carries out couriers for real:
// it writes the payload where the prompt says and runs the command, which runs
// workflows/deliver-step.mjs in a temp git repository with a fake omni first
// on PATH (fixtures/fake-omni.mjs) whose state is a JSON file, but for the
// sleep courier, which it answers at once. It answers the role agents from
// canned results by label, and writes each result to the file its prompt
// names, as the agent does, unless `written` says what to write instead:
// null for no file. An agent asked to write its result again writes the
// result its prompt gives, through `written` too. The runtime hands the
// script a copy of what an agent returned, and parallel a copy of what its
// thunks gave, as a runtime that serializes results would.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile, execFileSync, spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { fnv1a } from "./deliver-step.mjs";
import { readAgents } from "./measure-agents.mjs";

const SOURCE = readFileSync(fileURLToPath(new URL("./omni-deliver.js", import.meta.url)), "utf8");
const helper = fileURLToPath(new URL("./deliver-step.mjs", import.meta.url));
const fakeOmni = fileURLToPath(new URL("./fixtures/fake-omni.mjs", import.meta.url));
const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
// The runtime runs the script as the body of an async function, where meta is a constant.
const compile = (source) => new AsyncFunction("args", "agent", "parallel", "pipeline", "phase", "log", "Date", "Math", source.replace(/^export const meta = /m, "const meta = "));

// The runtime has no clock and no randomness: they would break resume.
const unavailable = (what) => () => {
  throw new Error(`${what} is not available in a workflow`);
};
const NoDate = new Proxy(Date, {
  apply: unavailable("Date()"),
  construct: unavailable("new Date()"),
  get: (target, name) => (name === "now" ? unavailable("Date.now()") : Reflect.get(target, name)),
});
const NoRandom = Object.create(Math, { random: { value: unavailable("Math.random()") } });

const git = (cwd, ...argv) => execFileSync("git", argv, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// Settles with the promise, or after ms at the latest.
const within = (promise, ms) => {
  let timer;
  return Promise.race([promise, new Promise((resolve) => { timer = setTimeout(resolve, ms); })]).finally(() => clearTimeout(timer));
};

function liveProcess(t) {
  const child = spawn("sleep", ["60"], { stdio: "ignore" });
  t.after(() => child.kill());
  return child.pid;
}

// A checkout `app` with one commit, the fake omni, and a live process standing
// in for the Claude Code session whose pid the lock holds.
function scratch(t, stories = [], threads = [], extra = {}) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "omni-deliver-")));
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

  const env = { ...process.env, PATH: `${bin}:${dirname(process.execPath)}:${process.env.PATH}`, FAKE_OMNI_STATE: statePath, OMNI_DELIVER_SESSION_PID: String(liveProcess(t)) };
  for (const name of ["FAKE_OMNI_FAIL", "FAKE_OMNI_DOWN", "FAKE_OMNI_HANG", "FAKE_OMNI_HANG_AFTER"]) delete env[name];
  const state = () => JSON.parse(readFileSync(statePath, "utf8"));
  return {
    root,
    repo,
    bin,
    env,
    state,
    story: (key) => state().stories.find((s) => s.key === key),
    worktree: (key) => join(root, "app-stories", key),
  };
}

function story(key, fields = {}) {
  return { key, title: `Story ${key}`, text: "", criteria: [], status: "backlog", blocked: false, holder: null, version: 1, decisions: [], links: [], blocked_by: [], files_planned: [], ...fields };
}

// A story that stays blocked: another that waits on it is clarity-checked but
// never picked, which keeps a clarity test out of delivery.
const blocker = () => story("US-9", { blocked: true });

// ---- the fake runtime --------------------------------------------------------------

const lineAfter = (text, marker) => {
  const at = text.indexOf(marker);
  return at === -1 ? null : text.slice(at + marker.length).split("\n")[0];
};
const between = (text, open, close) => {
  const at = text.indexOf(open);
  if (at === -1) return null;
  const from = at + open.length;
  return text.slice(from, text.indexOf(close, from));
};
// What a courier prompt asks for: the file to write, its line, and the command.
const fileIn = (prompt) => lineAfter(prompt, "Use the Write tool to create this file:\n");
const payloadIn = (prompt) => between(prompt, "\n<<<PAYLOAD\n", "\nPAYLOAD>>>\n");
const commandIn = (prompt) => lineAfter(prompt, "run this command exactly as written, once:\n");
// The file a role agent writes its result to, as its prompt names it.
const resultFileIn = (prompt) => lineAfter(prompt, "Before you return, use the Write tool to create this file:\n");

const sh = (command, dirs) => new Promise((resolve) => {
  execFile("/bin/sh", ["-c", command], { cwd: dirs.repo, env: dirs.env, maxBuffer: 64 * 1024 * 1024 }, (error, stdout) => {
    resolve({ code: error ? (Number.isInteger(error.code) ? error.code : 1) : 0, stdout: String(stdout) });
  });
});

// A courier as the agent carries it out: the Write tool, then the command in
// the checkout. `fault` makes it return nothing, write a payload one space
// off, change the output's body after the helper ran, or bring back lines a
// noisy shell printed around the helper's answer.
async function carryOut(dirs, prompt, fault) {
  if (fault === "nothing") return null;
  const file = fileIn(prompt);
  if (file !== null) {
    const payload = payloadIn(prompt);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, fault === "payload" ? payload.replace('{"opId":', '{ "opId":') : payload);
  }
  const { code, stdout } = await sh(commandIn(prompt), dirs);
  if (fault === "noisy") return { exitCode: code, output: `{"body": "a stray line printed before the answer"}\n(node:4242) Warning: a warning on standard output\n${stdout}a line printed after the answer\n` };
  if (fault !== "output") return { exitCode: code, output: stdout };
  const out = JSON.parse(stdout);
  return { exitCode: code, output: JSON.stringify({ body: { ...out.body, changed: true }, hash: out.hash }) };
}

let edits = 0;
function commit(cwd, file) {
  mkdirSync(dirname(join(cwd, file)), { recursive: true });
  writeFileSync(join(cwd, file), `${file} ${++edits}\n`);
  git(cwd, "add", file);
  git(cwd, "commit", "-q", "-m", `feat: ${file}`);
}

const worktreeIn = (prompt) => lineAfter(prompt, "## Your worktree\n").split(" (branch ")[0];
// The criteria of the story an agent's label names, numbered from 0, as the
// agent reads them from the board: no prompt carries them.
const criteriaIn = (call, dirs) => (dirs.story(call.label.split(":")[1]).criteria || []).map((c, index) => ({ index, criterion: typeof c === "string" ? c : c.text }));
const questionIn = (prompt) => lineAfter(prompt, "## The question\n");
// What a prompt says of the test obligations, which it names by number.
const owedIn = (prompt) => lineAfter(prompt, "## Acceptance criteria\n").split(" in the order the story gives them. ")[1];

// An implementer that commits a file in its worktree and is ready.
const built = (file, fields = {}) => async (call, serial) => {
  await serial(() => commit(worktreeIn(call.prompt), file));
  return { status: "ready", summary: `Built ${file}.`, decisions: [], commits: [], ...fields };
};
// A reviewer's verdict on every criterion of its prompt: met, but for those
// unmet names with their evidence and the test obligations it names.
const reviewed = (verdict, { unmet = {}, defects = [], obligations = [] } = {}) => (call, serial, dirs) => ({
  verdict,
  criteria: criteriaIn(call, dirs).map((c) => ({ ...c, obligation: obligations.includes(c.index), met: !(c.index in unmet), evidence: unmet[c.index] || "Seen in the diff" })),
  defects,
});
const vote = (pick, fields = {}) => ({ pick, quote: "", source: "", minor: false, ...fields });
// The answers of a story's council members, by reading: spec, code and user.
const councilOf = (key, spec, code, user) => ({ [`council:${key}:spec`]: spec, [`council:${key}:code`]: code, [`council:${key}:user`]: user });
const gap = (question, impact, fields = {}) => ({ criterion: "The badge has a colour", question, options: ["Red", "Blue"], recommendation: "Blue", reason: "\"warning colour\"", blocking: true, impact, ...fields });

// A verifier that ran every criterion of its prompt: ok, but for those failed
// names with what it saw.
const verified = (failed = {}) => (call, serial, dirs) => ({ criteria: criteriaIn(call, dirs).map((c) => ({ ...c, ok: !(c.index in failed), evidence: failed[c.index] || "Ran it" })) });
// A test author that is done; `fields` gives its commit, check and added tests.
const authored = (fields = {}) => ({ status: "done", commit: "", check: "", oracle: [], existingCoverage: [], added: [], reused: [], changedOrRemoved: [], notAdmitted: [], ...fields });
// A test author that commits a test file in the lane worktree, with a check.
const testsIn = (dirs, file, check = "true") => async (call, serial) => {
  await serial(() => commit(join(dirs.root, "app-stories", "_lane"), file));
  return authored({ check, added: [{ test: file, file, risk: "", overlapCheck: "", sensitivity: "" }] });
};

// What a role agent answers when its test gives it nothing: a clarity check
// with no gap, an implementer that commits a file, a reviewer that passes
// every criterion, a council member with no pick, a verifier that finds every
// criterion met, a test author that adds no test, and a final check that
// finds the run healthy.
const byDefault = (call, serial, dirs) => {
  const [role, key] = call.label.split(":");
  if (role === "clarity") return { testObligations: [], gaps: [] };
  if (role === "impl") return built(`${key}.txt`)(call, serial);
  if (role === "review") return reviewed("pass")(call, serial, dirs);
  if (role === "council") return vote(0);
  if (role === "verify") return verified()(call, serial, dirs);
  if (role === "tests") return authored();
  if (role === "final") return { healthy: true, journeys: [], report: "Nothing spans the stories." };
  throw new Error(`no answer for the agent ${call.label}`);
};

// A sleep courier answered at once, as the helper answers it, since a test
// cannot wait minutes: the seconds it asked for go in `sleeps`.
function slept(prompt, sleeps) {
  const payload = payloadIn(prompt);
  const { seconds } = JSON.parse(payload);
  sleeps.push(seconds);
  const body = { ok: true, received: fnv1a(payload), seconds };
  return { exitCode: 0, output: `${JSON.stringify({ body, hash: fnv1a(JSON.stringify(body)) })}\n` };
}

// The runtime's calls, with the agent fake behind them. Couriers run at once,
// as the runtime's agents do, or with `serial`, one at a time. The fake
// agents' own git commands go one at a time (serial), and in that line with
// the couriers when those are too.
function runtime(dirs, roles, fault, concurrent, written) {
  const calls = [];
  const logs = [];
  const phases = [];
  const sleeps = [];
  const queues = new Map(Object.entries(roles).map(([label, answers]) => [label, [...answers]]));
  let line = Promise.resolve();
  const serial = (fn) => {
    const next = line.then(fn);
    line = next.catch(() => {});
    return next;
  };
  const agent = async (prompt, opts = {}) => {
    const call = { prompt, ...opts };
    calls.push(call);
    await pause(0);
    if (String(opts.label).startsWith("courier:sleep")) return slept(prompt, sleeps);
    if (String(opts.label).startsWith("courier:")) {
      const r = await (concurrent ? carryOut(dirs, prompt, fault(call)) : serial(() => carryOut(dirs, prompt, fault(call))));
      call.output = r && r.output;
      return r;
    }
    if (String(opts.label).startsWith("again:")) {
      const file = lineAfter(prompt, "Use the Write tool to create this file:\n");
      const text = written(call, JSON.parse(between(prompt, "\n<<<RESULT\n", "\nRESULT>>>\n")));
      if (text !== null) {
        mkdirSync(dirname(file), { recursive: true });
        writeFileSync(file, text);
      }
      return { written: true };
    }
    const queue = queues.get(opts.label);
    const answer = queue && queue.length ? queue.shift() : byDefault;
    const r = await (typeof answer === "function" ? answer(call, serial, dirs) : answer);
    const file = resultFileIn(prompt);
    const text = r && file ? written(call, r) : null;
    if (text !== null) {
      mkdirSync(dirname(file), { recursive: true });
      writeFileSync(file, text);
    }
    return r === null || r === undefined ? r : structuredClone(r);
  };
  // A barrier: every thunk runs, and one that throws gives null.
  const parallel = (thunks) => Promise.all(thunks.map((thunk) => Promise.resolve().then(thunk).then((r) => (r === null || r === undefined ? r : structuredClone(r))).catch(() => null)));
  // No barrier: each item goes through the stages on its own, and a stage that
  // throws drops it to null.
  const pipeline = (items, ...stages) => Promise.all(items.map((item, index) =>
    stages.reduce((prev, stage) => prev.then((value) => stage(value, item, index)), Promise.resolve(item)).catch(() => null)));
  return { calls, logs, phases, sleeps, agent, parallel, pipeline, phase: (title) => phases.push(title), log: (message) => logs.push(message) };
}

// Runs the workflow to its end under the fake runtime, with the worktree's
// helper; gives what it returned or threw, and the runtime's record.
async function deliver(dirs, args = {}, roles = {}, { fault = () => null, source = SOURCE, concurrent = true, written = (call, r) => JSON.stringify(r) } = {}) {
  const rt = runtime(dirs, roles, fault, concurrent, written);
  let result = null;
  let error = null;
  try {
    result = await compile(source)({ feature: "board", helper, ...args }, rt.agent, rt.parallel, rt.pipeline, rt.phase, rt.log, NoDate, NoRandom);
  } catch (e) {
    error = e;
  }
  return { result, error, calls: rt.calls, logs: rt.logs, phases: rt.phases, sleeps: rt.sleeps };
}

let testOps = 0;
// A step of the run as a courier runs it, from the test: the payload file,
// then the helper with the payload's hash.
async function helperStep(dirs, run, name, fields = {}) {
  const payload = JSON.stringify({ opId: `test-${name}-${++testOps}`, ...fields });
  const file = join(dirs.repo, ".omni", "runs", run, "in", `test-${name}-${testOps}.json`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, payload);
  return JSON.parse((await sh(`node ${helper} ${name} --run ${run} --payload ${file} --hash ${fnv1a(payload)}`, dirs)).stdout).body;
}
// The next run's start, once a run has finished and given the lock up.
async function nextRun(dirs) {
  const op = `test-start-${++testOps}`;
  return JSON.parse((await sh(`node ${helper} start --op ${op} --feature board --hash ${fnv1a(JSON.stringify({ opId: op, feature: "board", force: false }))}`, dirs)).stdout).body.run;
}

const couriersOf = (calls) => calls.filter((c) => c.label.startsWith("courier:"));
const agentsOf = (calls, label) => calls.filter((c) => c.label === label);
// The stages the board was asked to claim the story for, in order.
const claimsOf = (dirs, key) => dirs.state().calls.filter((c) => c.argv[0] === "story" && c.argv[1] === "claim" && c.argv[2] === key).map((c) => c.argv[c.argv.indexOf("--stage") + 1]);
// The claims and moves the run sent for the story, in order: "claim <stage>" or "move <status>".
const timeline = (dirs, key) => dirs.state().calls
  .filter((c) => c.argv[0] === "story" && (c.argv[1] === "claim" || c.argv[1] === "move") && c.argv[2] === key)
  .map((c) => (c.argv[1] === "claim" ? `claim ${c.argv[c.argv.indexOf("--stage") + 1]}` : `move ${c.argv[c.argv.indexOf("--to") + 1]}`));
// The run's claims and moves in the order the board got them: "claim <key> <stage>" or "move <key> <status>".
const boardLog = (dirs) => dirs.state().calls
  .filter((c) => c.argv[0] === "story" && (c.argv[1] === "claim" || c.argv[1] === "move"))
  .map((c) => `${c.argv[1]} ${c.argv[2]} ${c.argv[c.argv.indexOf(c.argv[1] === "claim" ? "--stage" : "--to") + 1]}`);
// The most stories the board had in a stage at once: a claim for the stage
// counts one in, a move to one of `out` one out.
const peakIn = (dirs, stage, out) => {
  let now = 0;
  let peak = 0;
  for (const entry of boardLog(dirs)) {
    const [verb, , where] = entry.split(" ");
    if (verb === "claim" && where === stage) now++;
    if (verb === "move" && out.includes(where)) now--;
    peak = Math.max(peak, now);
  }
  return peak;
};
// The threads on the board besides the Run thread that start opens.
const questions = (dirs) => dirs.state().threads.filter((th) => !/^Run run-\d{8}-\d{6}-\d+$/.test(th.title));
const findingsOf = (dirs, key) => (dirs.story(key).findings || []).map((f) => [f.stage, f.text, f.files, f.resolved]);

// ---- the fake omni's phase rules ----------------------------------------------------

const omni = (dirs, ...argv) => spawnSync(join(dirs.bin, "omni"), argv, { cwd: dirs.repo, env: dirs.env, encoding: "utf8" });

test("the fake refuses a claim from a status that is not its stage's waiting state, changing nothing, and claims from the waiting state", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { status: "implemented" })]);

  const refused = omni(dirs, "story", "claim", "US-1", "--run", "r", "--stage", "review");
  const claimed = omni(dirs, "story", "claim", "US-2", "--run", "r", "--stage", "review");

  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /not_waiting/);
  assert.deepEqual(dirs.story("US-1"), story("US-1"));
  assert.equal(claimed.status, 0);
  assert.equal(dirs.story("US-2").status, "review");
  assert.deepEqual(dirs.story("US-2").holder, { run: "r", stage: "review" });
});

test("the fake's holder moving an implementing story to implemented clears the holder", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implementing", holder: { run: "r", stage: "implementing" } })]);

  const moved = omni(dirs, "story", "move", "US-1", "--run", "r", "--to", "implemented");

  assert.equal(moved.status, 0);
  assert.equal(dirs.story("US-1").status, "implemented");
  assert.equal(dirs.story("US-1").holder, null);
});

test("the fake's release returns each working story to its stage's waiting state with no holder", (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "implementing", holder: { run: "r", stage: "implementing" } }),
    story("US-2", { status: "review", holder: { run: "r", stage: "review" } }),
    story("US-3", { status: "testing", holder: { run: "r", stage: "testing" } }),
  ]);

  omni(dirs, "story", "release", "US-1", "--run", "r");
  omni(dirs, "story", "release", "US-2", "--run", "r");
  omni(dirs, "story", "release", "US-3", "--run", "r");

  assert.deepEqual(dirs.state().stories.map((s) => [s.key, s.status, s.holder, s.blocked]), [["US-1", "backlog", null, false], ["US-2", "implemented", null, false], ["US-3", "reviewed", null, false]]);
});

test("the fake's block on a story held in testing returns it to reviewed with no holder and blocked", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", holder: { run: "r", stage: "testing" } })]);

  omni(dirs, "story", "block", "US-1", "--run", "r");

  assert.equal(dirs.story("US-1").status, "reviewed");
  assert.equal(dirs.story("US-1").holder, null);
  assert.equal(dirs.story("US-1").blocked, true);
});

test("the fake leaves out of pickable a backlog story whose planned file a reviewed story changed, and lists it once that story is done", (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "reviewed", files_planned: ["src/other.rs"], files_changed: ["src/board.rs"] }),
    story("US-2", { files_planned: ["src/board.rs"] }),
  ]);

  const whileReviewed = omni(dirs, "story", "list", "--pickable").stdout;
  omni(dirs, "story", "move", "US-1", "--run", "r", "--to", "done");
  const onceDone = omni(dirs, "story", "list", "--pickable").stdout;

  assert.equal(whileReviewed, "");
  assert.equal(onceDone, "US-2\n");
});

// ---- the courier --------------------------------------------------------------------

test("the script's own fnv1a agrees with the helper's on ASCII, Unicode, emoji and CRLF strings", () => {
  const declared = SOURCE.match(/^function fnv1a\(text\) \{\n[\s\S]*?\n\}$/m);
  assert.ok(declared, "the script declares function fnv1a(text)");
  const scriptFnv1a = new Function(`${declared[0]}\nreturn fnv1a`)();

  for (const s of ["", "a", "foobar", '{"opId":"claim-US-1-1","key":"US-1"}', "naïve café", "日本語のテキスト", "😀 grinning 🎉", "line one\r\nline two\r\n"]) {
    assert.equal(scriptFnv1a(s), fnv1a(s), JSON.stringify(s));
  }
});

test("a courier is a haiku agent at low effort with the schema {exitCode, output}: it writes the payload to <root>/.omni/runs/<run>/in/<opId>.json and runs node <helper> <step> --run --payload --hash in a Bash call with the timeout at 600000; start goes by flags, its feature quoted and --force from args.force", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t);

  const { result, error, calls, phases } = await deliver(dirs, { feature: "Bob's board", force: true });

  assert.equal(error, null);
  const couriers = couriersOf(calls);
  assert.deepEqual(couriers.map((c) => c.label), ["courier:start", "courier:unblock-pass", "courier:clarity-pending", "courier:list", "courier:finish"]);
  for (const c of couriers) {
    assert.deepEqual([c.model, c.effort, c.agentType, c.schema.required], ["haiku", "low", "omni-courier", ["exitCode", "output"]]);
    assert.match(c.prompt, /\. Run the helper\. Use the Bash tool, with its timeout set to 600000, to run this command exactly as written, once:\n/);
  }
  const startHash = fnv1a(JSON.stringify({ opId: "start-1", feature: "Bob's board", force: true }));
  assert.deepEqual([fileIn(couriers[0].prompt), commandIn(couriers[0].prompt)], [null, `node ${helper} start --op start-1 --feature 'Bob'\\''s board' --force --hash ${startHash}`]);
  const file = join(dirs.repo, ".omni", "runs", result.run, "in", "unblock-pass-1.json");
  const payload = JSON.stringify({ opId: "unblock-pass-1" });
  assert.deepEqual([fileIn(couriers[1].prompt), payloadIn(couriers[1].prompt), commandIn(couriers[1].prompt)], [file, payload, `node ${helper} unblock-pass --run ${result.run} --payload ${file} --hash ${fnv1a(payload)}`]);
  assert.deepEqual([result.feature, result.branch, JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", result.run, "run.json"), "utf8")).feature], ["Bob's board", "deliver/bob-s-board", "Bob's board"]);
  assert.deepEqual(phases, ["Start", "Unblock", "Clarity", "Deliver", "Report"]);
});

test("without args.helper a courier runs ~/.claude/omni/deliver-step.mjs, for its shell to expand; a courier that returns nothing is sent 3 times more with the same prompt, and then the run stops, naming the step", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t);

  const { error, calls, logs } = await deliver(dirs, { helper: undefined }, {}, { fault: () => "nothing" });

  const couriers = couriersOf(calls);
  assert.equal(couriers.length, 4);
  assert.equal(new Set(couriers.map((c) => c.prompt)).size, 1);
  assert.match(commandIn(couriers[0].prompt), /^node ~\/\.claude\/omni\/deliver-step\.mjs start --op start-1 --feature 'board' --hash [0-9a-f]{8}$/);
  assert.equal(error.message, "courier:start: the courier returned nothing, after 3 retries");
  assert.deepEqual(logs, [1, 2, 3].map((i) => `courier:start: the courier returned nothing; trying again (${i} of 3)`));
});

test("a courier that returns nothing, miscopies the payload or changes the output is sent again with the same opId: the helper refuses the miscopy and answers the repeat from its record, so the claim is made once and the story goes on", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const faults = ["nothing", "payload", "output"];
  let tries = 0;
  const fault = (call) => (call.label === "courier:claim:US-1" && tries < faults.length ? faults[tries++] : null);

  const { result, error, calls, logs } = await deliver(dirs, {}, {}, { fault });

  assert.equal(error, null);
  const claims = agentsOf(calls, "courier:claim:US-1");
  assert.equal(claims.length, 6, "four tries of the first claim, then the claims for review and testing");
  assert.equal(new Set(claims.slice(0, 4).map((c) => c.prompt)).size, 1);
  assert.equal(fileIn(claims[0].prompt), join(dirs.repo, ".omni", "runs", result.run, "in", "claim-US-1-1.json"));
  assert.deepEqual(claimsOf(dirs, "US-1"), ["implementing", "review", "testing"]);
  const retries = logs.filter((l) => l.startsWith("courier:claim:US-1: "));
  assert.equal(retries.length, 3);
  assert.equal(retries[0], "courier:claim:US-1: the courier returned nothing; trying again (1 of 3)");
  assert.match(retries[1], /^courier:claim:US-1: the helper received another payload than the script sent: the payload hashes to [0-9a-f]{8}, not [0-9a-f]{8}: write it again exactly; trying again \(2 of 3\)$/);
  assert.equal(retries[2], "courier:claim:US-1: its output does not match its hash: the courier changed it; trying again (3 of 3)");
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("a step that fails is sent again with the same opId, which carries the failed write on: a question whose thread create failed opens one thread", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  let tries = 0;
  const fault = (call) => {
    if (call.label === "courier:thread-open:US-1") {
      if (tries++ === 0) dirs.env.FAKE_OMNI_FAIL = "thread";
      else delete dirs.env.FAKE_OMNI_FAIL;
    }
    return null;
  };
  const roles = { "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }] };

  const { error, calls, logs } = await deliver(dirs, {}, roles, { fault });

  assert.equal(error, null);
  const opens = agentsOf(calls, "courier:thread-open:US-1");
  assert.deepEqual([opens.length, opens[0].prompt === opens[1].prompt], [2, true]);
  assert.ok(logs.includes("courier:thread-open:US-1: thread-open failed: thread create failed; trying again (1 of 3)"), logs.join("\n"));
  const [question] = questions(dirs);
  assert.deepEqual([questions(dirs).length, question.title, question.posts.length], [1, "US-1: Which colour is the badge?", 1]);
  assert.deepEqual([dirs.story("US-1").blocked, dirs.story("US-1").links], [true, [{ thread_id: question.id, label: "question" }]]);
});

test("a courier's output may carry other lines around the helper's answer, as a shell or node prints them: the last line that starts as the answer does, with {\"body\":, is read", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t);

  const { result, error, logs } = await deliver(dirs, {}, {}, { fault: () => "noisy" });

  assert.equal(error, null);
  assert.match(result.run, /^run-\d{8}-\d{6}-\d+$/);
  assert.deepEqual(logs.filter((l) => l.includes("trying again")), []);
});

test("a step the helper refuses for good, with retry false, is not sent again: a start against the lock of a live run stops the run at once, after one courier", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t);
  const now = new Date().toISOString();
  mkdirSync(join(dirs.repo, ".omni"), { recursive: true });
  writeFileSync(join(dirs.repo, ".omni", "run.lock"), JSON.stringify({ run: "run-20261107-090000-1", feature: "billing", pid: liveProcess(t), op: "start-1", started: now, heartbeat: now }));

  const { error, calls, logs } = await deliver(dirs);

  assert.equal(couriersOf(calls).length, 1);
  assert.match(error.message, /^courier:start: start failed: the tree is held by run run-20261107-090000-1 for feature billing, .*start with force if that run is dead$/);
  assert.deepEqual(logs, []);
});

// ---- clarity ------------------------------------------------------------------------

test("clarity records a criterion that names a test as a test obligation and a minor gap's recommendation as agent decisions, sends the 3 blocking gaps of most impact to the council, scope first, then data and security, UX and technical, and defers the rest by the cap", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"], criteria: [{ text: "The badge has a colour", human_check: false }, { text: "A regression test covers the badge", human_check: false }] }), story("US-2"), blocker()]);
  const gaps = [
    gap("Which font is the badge in?", "technical"),
    gap("Who may see the badge?", "data_security"),
    gap("Does the badge show on phones?", "ux"),
    gap("Is the badge in this story at all?", "scope"),
    gap("What does the badge count?", "scope"),
    gap("Which shade of red?", "ux", { blocking: false, recommendation: "Crimson" }),
  ];
  const quoted = vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" });
  const roles = {
    "clarity:US-1": [{ testObligations: [1], gaps }],
    "clarity:US-2": [null],
    ...councilOf("US-1", [quoted, quoted, quoted], [vote(1), vote(1), vote(1)], [vote(1), vote(1), vote(1)]),
  };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const decisions = dirs.story("US-1").decisions;
  const deferred = "Blue (deferred by the cap: a story asks at most 3 blocking questions)";
  assert.deepEqual(decisions.slice(0, 4), [
    { question: 'Is "A regression test covers the badge" behaviour or a test?', choice: "treated as a test obligation: the lane's test author owns it, and the verifier skips it", by: "agent" },
    { question: "Which shade of red?", choice: "Crimson", by: "agent" },
    { question: "Does the badge show on phones?", choice: deferred, by: "agent" },
    { question: "Which font is the badge in?", choice: deferred, by: "agent" },
  ]);
  const asked = ["Is the badge in this story at all?", "What does the badge count?", "Who may see the badge?"];
  assert.deepEqual(agentsOf(calls, "council:US-1:spec").map((c) => questionIn(c.prompt)).sort(), asked.sort());
  assert.deepEqual(decisions.slice(4).map((d) => d.question).sort(), asked.sort());
  for (const d of decisions.slice(4)) assert.deepEqual([d.choice, d.by], ['Red (council: "Badges are red" (doc doc-spec node n1))', "agent"]);
  assert.deepEqual([questions(dirs), dirs.story("US-1").blocked], [[], false]);
  assert.deepEqual(result.clarity, [{ key: "US-1", obligations: 1, minor: 1, deferred: 2, settled: 3, threads: [] }]);
  assert.deepEqual(result.council, { settled: 3, majority: 0, asked: 0 });
  // A check that returned nothing records nothing, and keeps its story out of delivery.
  const seen = JSON.parse(readFileSync(join(dirs.repo, ".omni", "clarity.json"), "utf8"));
  assert.deepEqual([Object.keys(seen), claimsOf(dirs, "US-2")], [["US-1"], []]);
  assert.ok(logs.includes("US-2: the clarity check returned nothing; the story is not delivered in this run, and is checked again at the next start"), logs.join("\n"));
});

test("the council settles a question all three members pick the same option for when a quote supports it, or all three call it minor, as an agent decision with no thread; a split question, or an unquoted one not all call minor, opens a thread with each member's pick and evidence, and the majority's pick or else the asker's as the recommendation", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4", "US-5"];
  const dirs = scratch(t, [...keys.map((key) => story(key, { blocked_by: ["US-9"] })), blocker()]);
  const quoted = vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" });
  const roles = {
    ...Object.fromEntries(keys.map((key) => [`clarity:${key}`, [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }]])),
    ...councilOf("US-1", [quoted], [vote(1)], [vote(1)]),
    ...councilOf("US-2", [vote(2, { minor: true })], [vote(2, { minor: true })], [vote(2, { minor: true })]),
    ...councilOf("US-3", [quoted], [vote(2)], [vote(1)]),
    ...councilOf("US-4", [vote(1)], [vote(2)], [vote(0)]),
    ...councilOf("US-5", [vote(1)], [vote(1)], [vote(1, { minor: true })]),
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which colour is the badge?", choice: 'Red (council: "Badges are red" (doc doc-spec node n1))', by: "agent" }]);
  assert.deepEqual(dirs.story("US-2").decisions, [{ question: "Which colour is the badge?", choice: "Blue (council: all three members call the gap minor)", by: "agent" }]);
  assert.deepEqual(keys.map((key) => dirs.story(key).blocked), [false, false, true, true, true]);
  const threads = new Map(questions(dirs).map((th) => [th.id, th]));
  const bodyOf = (key) => threads.get(dirs.story(key).links[0].thread_id).posts[0].body;
  assert.deepEqual(["US-3", "US-4", "US-5"].map((key) => threads.get(dirs.story(key).links[0].thread_id).title), ["US-3: Which colour is the badge?", "US-4: Which colour is the badge?", "US-5: Which colour is the badge?"]);
  assert.equal(bodyOf("US-3"), `Which colour is the badge?

Criterion: The badge has a colour

Asked by the clarity check.

Options:
1. Red
2. Blue

The council: it did not settle this, in one round with each member on its own:
- The spec docs and decisions: option 1, Red; evidence: "Badges are red" (doc doc-spec node n1); minor: no
- The code: option 2, Blue; evidence: no evidence; minor: no
- What the user would expect: option 1, Red; evidence: no evidence; minor: no

Recommendation: Red (the council's majority)

Why: "warning colour"
`);
  assert.match(bodyOf("US-4"), /\nRecommendation: Blue \(the recommendation of the clarity check\)\n/);
  assert.match(bodyOf("US-5"), /- What the user would expect: option 1, Red; evidence: no evidence; minor: yes\n\nRecommendation: Red \(the council's majority\)\n/);
  assert.deepEqual(["US-3", "US-4", "US-5"].map((key) => dirs.story(key).decisions), [[], [], []]);
  assert.deepEqual(result.council, { settled: 2, majority: 0, asked: 3 });
});

test("the council settles a question two of the three members pick the same option for when each of the two quotes its source, as an agent decision that says 2 of 3 and names the third member's pick, and counts it as settled by a majority", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3"];
  const dirs = scratch(t, [...keys.map((key) => story(key, { blocked_by: ["US-9"] })), blocker()]);
  const spec = vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" });
  const code = vote(1, { quote: "const BADGE = RED", source: "src/badge.rs:4" });
  const roles = {
    ...Object.fromEntries(keys.map((key) => [`clarity:${key}`, [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }]])),
    ...councilOf("US-1", [spec], [code], [vote(2, { quote: "Blue is calm", source: "doc doc-ux node n2" })]),
    ...councilOf("US-2", [spec], [vote(0)], [vote(1, { quote: "Users expect red", source: "doc doc-ux node n3" })]),
    ...councilOf("US-3", [null], [code], [vote(1, { quote: "Users expect red", source: "doc doc-ux node n3" })]),
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(keys.map((key) => dirs.story(key).decisions), [
    [{ question: "Which colour is the badge?", choice: 'Red (council, 2 of 3: "Badges are red" (doc doc-spec node n1); "const BADGE = RED" (src/badge.rs:4); dissent: What the user would expect: option 2, Blue)', by: "agent" }],
    [{ question: "Which colour is the badge?", choice: 'Red (council, 2 of 3: "Badges are red" (doc doc-spec node n1); "Users expect red" (doc doc-ux node n3); dissent: The code: no option)', by: "agent" }],
    [{ question: "Which colour is the badge?", choice: 'Red (council, 2 of 3: "const BADGE = RED" (src/badge.rs:4); "Users expect red" (doc doc-ux node n3); dissent: The spec docs and decisions: returned nothing)', by: "agent" }],
  ]);
  assert.deepEqual([questions(dirs), keys.map((key) => dirs.story(key).blocked)], [[], [false, false, false]]);
  assert.deepEqual(result.council, { settled: 3, majority: 3, asked: 0 });
});

test("a question two members pick the same option for, but only one of them with a quote, goes to the human, with the majority's pick as the recommendation, and counts as asked, not settled by a majority", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  const roles = {
    "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }],
    ...councilOf("US-1", [vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" })], [vote(1, { quote: "", source: "src/badge.rs:4" })], [vote(2, { quote: "Blue is calm", source: "doc doc-ux node n2" })]),
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual([dirs.story("US-1").decisions, dirs.story("US-1").blocked], [[], true]);
  const [thread] = questions(dirs);
  assert.match(thread.posts[0].body, /\n- The code: option 1, Red; evidence: no evidence; minor: no\n[\s\S]*\nRecommendation: Red \(the council's majority\)\n/);
  assert.deepEqual(result.council, { settled: 0, majority: 0, asked: 1 });
});

test("a question each member picks a different option for goes to the human, though every member quotes its source, and counts as asked, not settled by a majority", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  const roles = {
    "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux", { options: ["Red", "Blue", "Green"] })] }],
    ...councilOf("US-1", [vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" })], [vote(2, { quote: "const BADGE = BLUE", source: "src/badge.rs:4" })], [vote(3, { quote: "Green means go", source: "doc doc-ux node n2" })]),
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual([dirs.story("US-1").decisions, dirs.story("US-1").blocked], [[], true]);
  const [thread] = questions(dirs);
  assert.match(thread.posts[0].body, /\nRecommendation: Blue \(the recommendation of the clarity check\)\n/);
  assert.deepEqual(result.council, { settled: 0, majority: 0, asked: 1 });
});

test("a story's clarity councils write their answers one at a time: two questions sent to the human leave two threads linked from the story, though its reads are slow", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  // Each read of the story takes 300 ms: thread-opens at once would each link
  // the story from what they read, and the second would drop the first's.
  Object.assign(dirs.env, { FAKE_OMNI_HANG_AFTER: "story read US-1", FAKE_OMNI_HANG_MS: "300" });
  const roles = {
    "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "scope"), gap("Which size is the badge?", "ux")] }],
    ...councilOf("US-1", [vote(1), vote(1)], [vote(2), vote(2)], [vote(0), vote(0)]),
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const threads = questions(dirs);
  assert.deepEqual(threads.map((th) => th.title).sort(), ["US-1: Which colour is the badge?", "US-1: Which size is the badge?"]);
  assert.deepEqual(dirs.story("US-1").links.map((l) => l.thread_id).sort(), threads.map((th) => th.id).sort());
  assert.deepEqual([dirs.story("US-1").blocked, result.clarity[0].threads.length], [true, 2]);
});

test("every clarity council of a story is waited for, even when another one's answer cannot be written: what was settled stays, and the story is not recorded, so the next start checks it again", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  let sitting = 0;
  // The first question's members take a second; the second's are at once, and its thread cannot be created.
  const slow = (answer) => async () => {
    sitting++;
    await pause(1000);
    sitting--;
    return answer;
  };
  const quoted = vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" });
  const roles = {
    "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "scope"), gap("Which size is the badge?", "ux")] }],
    ...councilOf("US-1", [slow(quoted), vote(1)], [slow(vote(1)), vote(2)], [slow(vote(1)), vote(0)]),
  };
  const fault = (call) => {
    if (call.label === "courier:thread-open:US-1") dirs.env.FAKE_OMNI_FAIL = "thread";
    return null;
  };

  const { result, error, logs } = await deliver(dirs, {}, roles, { fault });

  assert.equal(error, null);
  assert.equal(sitting, 0, "no council sits on after the run");
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which colour is the badge?", choice: 'Red (council: "Badges are red" (doc doc-spec node n1))', by: "agent" }]);
  assert.deepEqual([questions(dirs), result.clarity], [[], []]);
  assert.ok(logs.some((l) => l.startsWith('US-1: the clarity check failed: a question was neither settled nor asked: "Which size is the badge?": ')), logs.join("\n"));
  delete dirs.env.FAKE_OMNI_FAIL;
  assert.deepEqual((await helperStep(dirs, await nextRun(dirs), "clarity-pending")).keys, ["US-1"]);
});

test("at most 2 councils run at once", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4"];
  const dirs = scratch(t, [...keys.map((key) => story(key, { blocked_by: ["US-9"] })), blocker()]);
  const members = new Map();
  let peak = 0;
  // Each member takes a second; a council runs while one of its members does.
  const slow = (answer) => async (call) => {
    const key = call.label.split(":")[1];
    members.set(key, (members.get(key) || 0) + 1);
    peak = Math.max(peak, [...members.values()].filter((n) => n > 0).length);
    await pause(1000);
    members.set(key, members.get(key) - 1);
    return answer;
  };
  const quoted = vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" });
  const roles = Object.assign(
    Object.fromEntries(keys.map((key) => [`clarity:${key}`, [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }]])),
    ...keys.map((key) => councilOf(key, [slow(quoted)], [slow(vote(1))], [slow(vote(1))])),
  );

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.equal(peak, 2);
  assert.equal(calls.filter((c) => c.label.startsWith("council:")).length, 12);
  assert.deepEqual(keys.map((key) => dirs.story(key).decisions.length), [1, 1, 1, 1]);
  assert.deepEqual(result.council, { settled: 4, majority: 0, asked: 0 });
});

// ---- deliver: implement and review ---------------------------------------------------------

test("a story is claimed, gets its worktree and an implementer on sonnet, records its files, moves to implemented, is claimed for review, and a reviewer on opus meeting every criterion moves it to reviewed, from where the lane lands it on deliver/<slug>", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }] })]);
  const roles = { "impl:US-1": [built("src/badge.rs")] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(timeline(dirs, "US-1"), ["claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move done"]);
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.blocked, done.files_changed], ["done", null, false, ["src/badge.rs"]]);
  assert.deepEqual([agentsOf(calls, "impl:US-1").map((c) => c.model), agentsOf(calls, "review:US-1").map((c) => c.model)], [["sonnet"], ["opus"]]);
  assert.match(agentsOf(calls, "impl:US-1")[0].prompt, new RegExp(`\n## Your worktree\n${dirs.worktree("US-1")} \\(branch story/US-1, cut from deliver/board\\)`));
  assert.deepEqual([git(dirs.repo, "log", "-1", "--format=%s", "refs/heads/deliver/board").trim(), existsSync(dirs.worktree("US-1"))], ["feat: src/badge.rs", false]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
  assert.equal(git(dirs.repo, "rev-parse", "--abbrev-ref", "HEAD").trim(), "main");
});

test("an implementer that returns nothing is asked again after 30 s and after 2 minutes, and then has its story released with no attempt used; one that is blocked opens a thread with its reason, which blocks the story; one that is ready has its decisions recorded as the agent's", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);
  const roles = {
    "impl:US-1": [null, null, null],
    "impl:US-2": [{ status: "blocked", summary: "The badge API is missing from the spec." }],
    "impl:US-3": [built("src/badge.rs", { decisions: [{ question: "Which file holds the badge?", choice: "src/badge.rs" }] })],
  };

  const { result, error, calls, logs, sleeps } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const outcome = (key) => result.outcomes.find((o) => o.key === key);
  assert.deepEqual(outcome("US-1"), { key: "US-1", outcome: "no_result", detail: "the implementer returned nothing, 3 times; the story is released without using an attempt" });
  assert.deepEqual([agentsOf(calls, "impl:US-1").length, sleeps], [3, [30, 120]]);
  assert.ok(logs.includes("US-1: the implementer returned nothing; asking again in 30 s, with no attempt used"), logs.join("\n"));
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").holder, dirs.story("US-1").blocked, dirs.story("US-1").findings || []], ["backlog", null, false, []]);
  const [blocked] = questions(dirs);
  assert.deepEqual([blocked.title, blocked.posts[0].body, outcome("US-2")], ["US-2: the implementer is blocked", "The badge API is missing from the spec.\n", { key: "US-2", outcome: "blocked", thread: blocked.id }]);
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").blocked, dirs.story("US-2").links], ["backlog", true, [{ thread_id: blocked.id, label: "question" }]]);
  assert.deepEqual(dirs.story("US-3").decisions, [{ question: "Which file holds the badge?", choice: "src/badge.rs", by: "agent" }]);
  assert.equal(dirs.story("US-3").status, "done");
});

test("a story whose delivery throws is released and gets a thread with the error, which blocks it, so the human hears and no run cycles on it: here an implementer renamed its branch, which leaves no story/<key> for its worktree to go back on, and the helper refuses that for good, so its courier is not sent again", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        commit(wt, "src/badge.rs");
        git(wt, "branch", "-q", "-m", "side");
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }],
    "review:US-1": [reviewed("fail", { unmet: { 0: "Pink" } })],
  };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const [stopped] = questions(dirs);
  assert.equal(stopped.title, "US-1: delivery stopped on an error");
  assert.match(stopped.posts[0].body, /^The workflow stopped delivering US-1 on this error:\n\ncourier:worktree:US-1: worktree failed: \S+ is not a worktree of \S+ on story\/US-1; move it away\n\nThe story is back in its waiting state, and blocked until this thread is answered; answer it once the cause is fixed\.\n$/);
  assert.equal(agentsOf(calls, "courier:worktree:US-1").length, 2, "one for each attempt, and no retry of the refusal");
  assert.deepEqual(logs.filter((l) => l.startsWith("courier:worktree:US-1: ")), []);
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.blocked, done.links], ["backlog", null, true, [{ thread_id: stopped.id, label: "question" }]]);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome, o.thread]), [["US-1", "failed", stopped.id]]);
});

test("a story whose delivery throws is released even when its thread cannot be opened, so that no run finds it held", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        commit(wt, "src/badge.rs");
        git(wt, "branch", "-q", "-m", "side");
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }],
    "review:US-1": [reviewed("fail", { unmet: { 0: "Pink" } })],
  };
  const fault = (call) => {
    if (call.label === "courier:thread-open:US-1") dirs.env.FAKE_OMNI_FAIL = "thread";
    return null;
  };

  const { result, error, logs } = await deliver(dirs, {}, roles, { fault });

  assert.equal(error, null);
  assert.deepEqual(questions(dirs), []);
  assert.ok(logs.some((l) => l.startsWith("US-1: the thread about the error could not be opened: ")), logs.join("\n"));
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.blocked], ["backlog", null, false]);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome, o.thread]), [["US-1", "failed", undefined]]);
});

test("a second story stopped by the same error, its key aside, stops the run taking stories with one thread for the run: the first story has its own thread, and every later failure is only released", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4", "US-5"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  // A lasting cause: the deliver branch is deleted before any story is delivered.
  const roles = { "clarity:US-1": [async (call, serial) => {
    await serial(() => git(dirs.repo, "update-ref", "-d", "refs/heads/deliver/board"));
    return { testObligations: [], gaps: [] };
  }] };

  const { result, error, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const blocked = keys.filter((key) => dirs.story(key).blocked);
  assert.equal(blocked.length, 1, keys.map((key) => `${key} ${dirs.story(key).blocked}`).join(", "));
  const threads = questions(dirs);
  const own = threads.find((th) => th.title === `${blocked[0]}: delivery stopped on an error`);
  const run = threads.find((th) => th.title === `Run ${result.run}: stories stop on the same error`);
  assert.deepEqual([threads.length, !!own, !!run], [2, true, true], threads.map((th) => th.title).join("\n"));
  assert.deepEqual(dirs.story(blocked[0]).links, [{ thread_id: own.id, label: "question" }]);
  const [, first, second] = run.posts[0].body.match(/^(US-\d) and (US-\d) stopped on the same error:\n\ncourier:worktree:US-\d: worktree failed: /);
  assert.equal(first, blocked[0]);
  assert.equal(result.halted, `${first} and ${second} stopped on the same error`);
  assert.ok(result.notes.includes(`The run took no new story once ${first} and ${second} stopped on the same error (thread ${run.id}).`), result.notes.join("\n"));
  assert.ok(logs.includes(`The run takes no new story: ${first} and ${second} stopped on the same error`), logs.join("\n"));
  for (const key of keys.filter((k) => k !== first)) assert.deepEqual([dirs.story(key).status, dirs.story(key).holder, dirs.story(key).blocked], ["backlog", null, false], key);
  assert.deepEqual(claimsOf(dirs, "US-5"), []);
  assert.ok(result.outcomes.every((o) => o.outcome === "failed"), JSON.stringify(result.outcomes));
});

test("the breaker takes a story's key out of an error only where it stands as a word of its own: two stories stopped by an error that names the branch deliver/us-10, one of them us-1, trip it", { timeout: 60000 }, async (t) => {
  const keys = ["us-1", "us-2"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  // A lasting cause whose error holds us-1 inside a longer word: the deliver branch of the feature us-10 is deleted.
  const roles = { "clarity:us-1": [async (call, serial) => {
    await serial(() => git(dirs.repo, "update-ref", "-d", "refs/heads/deliver/us-10"));
    return { testObligations: [], gaps: [] };
  }] };

  const { result, error } = await deliver(dirs, { feature: "us-10" }, roles);

  assert.equal(error, null);
  const threads = questions(dirs).map((th) => th.title);
  const [first] = keys.filter((key) => dirs.story(key).blocked);
  const second = keys.find((key) => key !== first);
  assert.deepEqual([threads.sort(), result.halted], [[`Run ${result.run}: stories stop on the same error`, `${first}: delivery stopped on an error`].sort(), `${first} and ${second} stopped on the same error`]);
});

test("a story whose implementer committed on another branch reaches an implementer again: lane-prepare's wrong_branch uses an attempt, the worktree step puts the worktree back on story/<key>, and the next implementer is told which commits to cherry-pick, which land; no story stops on an error", { timeout: 120000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  const side = async (call, serial) => {
    const key = call.label.split(":")[1];
    await serial(() => {
      const wt = worktreeIn(call.prompt);
      git(wt, "checkout", "-q", "-b", `side-${key}`);
      commit(wt, `${key}.txt`);
    });
    return { status: "ready", summary: "Built.", decisions: [], commits: [] };
  };
  // Brings over the commits its prompt names, with git cherry-pick.
  const picks = async (call, serial) => {
    const named = call.prompt.match(/ with git cherry-pick, oldest first: (.+)\.\n/)[1].split("; ").map((c) => c.split(" ")[0]);
    await serial(() => git(worktreeIn(call.prompt), "cherry-pick", ...named));
    return { status: "ready", summary: "Picked.", decisions: [], commits: [] };
  };
  const roles = { "impl:US-1": [side, picks], "impl:US-2": [side, picks] };

  const { result, error, calls } = await deliver(dirs, { implementers: 2 }, roles);

  assert.equal(error, null);
  for (const key of ["US-1", "US-2"]) {
    const sideCommit = git(dirs.repo, "log", "-1", "--format=%h %s", `side-${key}`).trim();
    assert.deepEqual(findingsOf(dirs, key), [["testing", `[attempt 1] ${dirs.worktree(key)} has the branch side-${key} checked out, not story/${key}: only story/${key} lands, so its 1 commit not on story/${key} would be lost (${sideCommit}). Commit on story/${key}`, [], true]], key);
    const impls = agentsOf(calls, `impl:${key}`);
    assert.equal(impls.length, 2, key);
    assert.ok(impls[1].prompt.includes(`\n## Your worktree\n${dirs.worktree(key)} (branch story/${key}, cut from deliver/board). Work only there: cd into it first, and run every command there.\n${dirs.worktree(key)} had the branch side-${key} checked out, not story/${key}, which is what lands: the workflow checked story/${key} out again. Bring over its 1 commit not on story/${key} with git cherry-pick, oldest first: ${sideCommit}.\n`), impls[1].prompt);
    assert.equal(git(dirs.repo, "show", `refs/heads/deliver/board:${key}.txt`), git(dirs.repo, "show", `side-${key}:${key}.txt`), key);
  }
  assert.deepEqual([result.halted, questions(dirs), result.outcomes.map((o) => o.outcome)], [null, [], ["done", "done", "done"]]);
});

test("what the worktree step found when a review started reaches the next implementer: a story waiting in implemented, whose worktree had another branch checked out, is back on story/<key> for its review, and the implementer after the review fails is told which commits to cherry-pick, which land", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented", criteria: [{ text: "The badge is red", human_check: false }] })]);
  const wt = dirs.worktree("US-1");
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-1", wt);
  commit(wt, "US-1.txt");
  git(wt, "checkout", "-q", "-b", "side");
  commit(wt, "side.txt");
  const sideCommit = git(wt, "log", "-1", "--format=%h %s").trim();
  // Brings over the commits its prompt names, with git cherry-pick.
  const picks = async (call, serial) => {
    const named = call.prompt.match(/ with git cherry-pick, oldest first: (.+)\.\n/)[1].split("; ").map((c) => c.split(" ")[0]);
    await serial(() => git(worktreeIn(call.prompt), "cherry-pick", ...named));
    return { status: "ready", summary: "Picked.", decisions: [], commits: [] };
  };
  const roles = { "review:US-1": [reviewed("fail", { unmet: { 0: "No side.txt in the diff" } }), reviewed("pass")], "impl:US-1": [picks] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const [impl] = agentsOf(calls, "impl:US-1");
  assert.ok(impl.prompt.includes(`\n${wt} had the branch side checked out, not story/US-1, which is what lands: the workflow checked story/US-1 out again. Bring over its 1 commit not on story/US-1 with git cherry-pick, oldest first: ${sideCommit}.\n`), impl.prompt);
  assert.equal(git(dirs.repo, "show", "refs/heads/deliver/board:side.txt"), git(dirs.repo, "show", "side:side.txt"));
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("a review that passes clears what the worktree step found when it started: the implementer after a lane failure is not told to cherry-pick the commits the review went without", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented", criteria: [{ text: "The badge is red", human_check: false }] })]);
  const wt = dirs.worktree("US-1");
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-1", wt);
  commit(wt, "US-1.txt");
  git(wt, "checkout", "-q", "-b", "side");
  commit(wt, "side.txt");
  const roles = { "verify:US-1": [verified({ 0: "The badge is blue" })] };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.ok(logs.some((l) => l.startsWith(`US-1: ${wt} had the branch side checked out, not story/US-1`)), logs.join("\n"));
  const [impl] = agentsOf(calls, "impl:US-1");
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", '[attempt 1] Criterion not met when run: "The badge is red". The badge is blue', [], true]]);
  assert.doesNotMatch(impl.prompt, /cherry-pick/);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("a story waiting in implemented starts at its review; a criterion that names a test is a test obligation, marked so, that the review never judges, whether clarity recorded it or not", { timeout: 60000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the badge", human_check: false }];
  const dirs = scratch(t, [story("US-1", { status: "implemented", criteria }), story("US-2", { criteria })]);
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-1", dirs.worktree("US-1"));
  commit(dirs.worktree("US-1"), "src/badge.rs");
  const roles = {
    // US-1 came before clarity, so no decision: the criterion names a test all the same.
    "review:US-1": [reviewed("pass", { unmet: { 1: "No test in the diff" } })],
    "clarity:US-2": [{ testObligations: [1], gaps: [] }],
    // US-2's reviewer judges the obligation clarity recorded; the judgement is not read.
    "review:US-2": [reviewed("pass", { unmet: { 1: "No test in the diff" } })],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual([timeline(dirs, "US-1"), dirs.story("US-1").status], [["claim review", "move reviewed", "claim testing", "move done"], "done"]);
  assert.deepEqual(agentsOf(calls, "impl:US-1"), []);
  assert.equal(dirs.story("US-2").status, "done");
  for (const key of ["US-1", "US-2"]) assert.equal(owedIn(agentsOf(calls, `review:${key}`)[0].prompt), "Criterion 1 is a test obligation.");
  assert.deepEqual([dirs.story("US-1").findings || [], dirs.story("US-2").findings || []], [[], []]);
  assert.deepEqual(result.outcomes.map((o) => o.outcome), ["done", "done"]);
});

test("criteria are numbered from 0 in the review's schema, which wants an entry for each; a review that leaves one without a verdict is malformed and asked again, never taken as met, and a story whose reviewer stays malformed is released with no attempt used", { timeout: 60000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "The badge counts", human_check: false }];
  const dirs = scratch(t, [story("US-1", { criteria }), story("US-2", { criteria })]);
  // Numbered from 1: the entry for 0 is missing, and the one for 2 is past the end.
  const fromOne = (call, serial, dirs) => ({ verdict: "pass", criteria: criteriaIn(call, dirs).map((c) => ({ ...c, index: c.index + 1, obligation: false, met: true, evidence: "Seen" })), defects: [] });
  const roles = { "review:US-1": [fromOne, reviewed("pass")], "review:US-2": [fromOne, fromOne, fromOne] };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const schema = agentsOf(calls, "review:US-1")[0].schema.properties.criteria;
  assert.deepEqual([schema.minItems, schema.maxItems, schema.items.properties.index.minimum, schema.items.properties.index.maximum], [2, 2, 0, 1]);
  assert.match(schema.items.properties.index.description, /counting from 0/);
  assert.deepEqual([agentsOf(calls, "review:US-1").length, dirs.story("US-1").status], [2, "done"]);
  assert.ok(logs.includes("US-1: the review gave no verdict on criteria 0; asking again"), logs.join("\n"));
  assert.equal(agentsOf(calls, "review:US-2").length, 3);
  assert.deepEqual(result.outcomes.find((o) => o.key === "US-2"), { key: "US-2", outcome: "no_result", detail: "the reviewer gave no verdict on criteria 0, 3 times; the story is released without using an attempt" });
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").holder, dirs.story("US-2").findings || []], ["implemented", null, []]);
});

test("the run takes the pickable stories in the helper's order: one with a worktree from an earlier attempt before one of higher priority, and its implementer goes on with that worktree", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { priority: 1 }), story("US-2")]);
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-2", dirs.worktree("US-2"));
  commit(dirs.worktree("US-2"), "src/half.rs");

  const { error, calls } = await deliver(dirs, { implementers: 1 });

  assert.equal(error, null);
  assert.deepEqual(boardLog(dirs).filter((e) => e.endsWith(" implementing")), ["claim US-2 implementing", "claim US-1 implementing"]);
  assert.match(lineAfter(agentsOf(calls, "impl:US-2")[0].prompt, "## Your task now\n"), /A worktree of an earlier attempt is in place/);
});

test("with storyLimit, the run takes at most that many distinct stories", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const roles = { "review:US-1": [reviewed("fail", { defects: [{ text: "Pink", files: [], criterion: "" }] }), reviewed("pass")] };

  const { result, error, logs } = await deliver(dirs, { storyLimit: 1 }, roles);

  assert.equal(error, null);
  assert.deepEqual([timeline(dirs, "US-1").filter((e) => e === "claim implementing").length, claimsOf(dirs, "US-2")], [2, []]);
  assert.ok(logs.includes("The run has taken 1 of its storyLimit of 1: it takes no new story"), logs.join("\n"));
  assert.deepEqual([result.storyLimit, result.outcomes.map((o) => o.key)], [1, ["US-1"]]);
});

test("a review fails on a criterion not met, even with the verdict pass, and on every defect it lists: each becomes a review finding of the attempt, and the story goes back for the next attempt, to the backlog and claimed again by one rework step, until a review meets every criterion with no defect", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }, { text: "The badge counts", human_check: false }] })]);
  const roles = {
    "review:US-1": [
      reviewed("pass", { unmet: { 1: "No count at src/badge.rs:3" } }),
      reviewed("fail", { defects: [{ text: "A new test file in the diff", files: ["badge.test.rs"], criterion: "" }, { text: "The count is off by one", files: ["src/badge.rs"], criterion: "The badge counts" }] }),
      reviewed("pass"),
    ],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  // Each send-back goes back to the backlog and is claimed again in one courier.
  const reworks = agentsOf(calls, "courier:rework:US-1");
  assert.deepEqual([reworks.length, agentsOf(calls, "courier:move:US-1").map(payloadOf).filter((p) => p.to === "backlog")], [2, []]);
  assert.deepEqual(findingsOf(dirs, "US-1"), [
    ["review", '[attempt 1] Criterion not met: "The badge counts". No count at src/badge.rs:3', [], true],
    ["review", "[attempt 2] A new test file in the diff", ["badge.test.rs"], true],
    ["review", '[attempt 2] The count is off by one (criterion: "The badge counts")', ["src/badge.rs"], true],
  ]);
  assert.deepEqual(timeline(dirs, "US-1"), [
    "claim implementing", "move implemented", "claim review", "move backlog",
    "claim implementing", "move implemented", "claim review", "move backlog",
    "claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move done",
  ]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("the script decides a review from its verdicts on the criteria and its defects, not from the reviewer's verdict: a test obligation's verdict and a defect that names only an obligation are not read, and a reviewer's obligation true counts for no other criterion", { timeout: 60000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the badge", human_check: false }];
  const dirs = scratch(t, [story("US-1", { criteria }), story("US-2", { criteria }), story("US-3", { criteria })]);
  const obligation = { testObligations: [1], gaps: [] };
  const roles = {
    "clarity:US-1": [obligation],
    "clarity:US-2": [obligation],
    // Judges the obligation clarity recorded, and fails the story for it.
    "review:US-1": [reviewed("fail", { unmet: { 1: "No test in the diff" } })],
    // Leaves the obligation unjudged, and files a defect that names it.
    "review:US-2": [reviewed("fail", { obligations: [1], defects: [{ text: "No regression test for the badge", files: [], criterion: "A regression test covers the badge" }] })],
    // Calls a behaviour an obligation, which leaves it unjudged.
    "review:US-3": [reviewed("pass", { unmet: { 0: "Not judged: a test obligation" }, obligations: [0] })],
  };

  const { error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const once = ["claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move done"];
  assert.deepEqual(["US-1", "US-2"].map((key) => [timeline(dirs, key), findingsOf(dirs, key)]), [[once, []], [once, []]]);
  assert.deepEqual(findingsOf(dirs, "US-3"), [["review", '[attempt 1] Criterion not met: "The badge is red". Not judged: a test obligation', [], true]]);
  assert.deepEqual(timeline(dirs, "US-3"), ["claim implementing", "move implemented", "claim review", "move backlog", ...once]);
});

test("a story clarity has checked has its test obligations from clarity's decisions alone: a behaviour criterion with the word test in it is judged by the reviewer and the verifier; a story clarity never checked is read for a test as the thing delivered, never for a bare test or suite", { timeout: 120000 }, async (t) => {
  const button = { text: "The Test connection button shows Connected", human_check: false };
  // Behaviour that reads as a test to add, which clarity, having checked the story, did not record as one.
  const drive = { text: "A user can add a test drive to the booking", human_check: false };
  const criteria = [button, { text: "The suite of badges loads", human_check: false }, { text: "Tests cover the badge count", human_check: false }];
  const dirs = scratch(t, [story("US-1", { criteria: [button, drive] }), story("US-2", { status: "implemented", criteria })]);
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-2", dirs.worktree("US-2"));
  commit(dirs.worktree("US-2"), "src/badge.rs");
  const roles = { "review:US-1": [reviewed("fail", { unmet: { 0: "It shows nothing" } })], "verify:US-1": [verified({ 0: "The button shows an error" })] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [
    ["review", '[attempt 1] Criterion not met: "The Test connection button shows Connected". It shows nothing', [], true],
    ["testing", '[attempt 2] Criterion not met when run: "The Test connection button shows Connected". The button shows an error', [], true],
  ]);
  assert.equal(owedIn(agentsOf(calls, "review:US-1")[0].prompt), "No criterion is a test obligation.");
  assert.equal(owedIn(agentsOf(calls, "review:US-2")[0].prompt), "Criterion 2 is a test obligation.");
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]).sort(), [["US-1", "done"], ["US-2", "done"]]);
});

test("a story version the old runner kept in the clarity cache is not a clarity check of this workflow's, which records test obligations: a story implemented before this run is read for a test as the thing delivered, with that cache as without one", { timeout: 120000 }, async (t) => {
  const criteria = [{ text: "A regression test covers the badge colour", human_check: false }, { text: "The badge is red", human_check: false }];
  for (const cache of [null, { "US-1": 7 }]) {
    const dirs = scratch(t, [story("US-1", { status: "implemented", criteria })]);
    if (cache) {
      mkdirSync(join(dirs.repo, ".omni"), { recursive: true });
      writeFileSync(join(dirs.repo, ".omni", "clarity.json"), JSON.stringify(cache));
    }

    const { error, calls } = await deliver(dirs);

    assert.equal(error, null);
    for (const role of ["review", "verify"]) assert.equal(owedIn(agentsOf(calls, `${role}:US-1`)[0].prompt), "Criterion 0 is a test obligation.", `${role} with ${JSON.stringify(cache)}`);
  }
});

test("from attempt 2 the implementer runs on implRetry, in the same worktree; it reads the story with omni story read as this run's impl, and its prompt counts the open findings by kind, those since the human's latest answer, those that used no attempt and those before it, never quoting one", { timeout: 60000 }, async (t) => {
  const answered = { stage: "testing", text: "[attempt 1] The badge was blue", files: [], resolved: true };
  const conflict = { stage: "testing", text: "rebase conflict in README: rebase it onto deliver/board", files: ["README"], resolved: false };
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }], findings: [answered, conflict] })]);
  const defect = (text, file) => reviewed("fail", { defects: [{ text, files: [file], criterion: "" }] });
  const roles = { "review:US-1": [defect("The badge is pink", "src/badge.rs"), defect("The badge has no border", "src/border.rs"), reviewed("pass")] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const impls = agentsOf(calls, "impl:US-1");
  assert.deepEqual(impls.map((c) => c.model), ["sonnet", "opus", "opus"]);
  assert.deepEqual(impls.map((c) => worktreeIn(c.prompt)), [1, 2, 3].map(() => dirs.worktree("US-1")));
  assert.deepEqual(impls.map((c) => lineAfter(c.prompt, "## Your task now\n")), [
    "This is attempt 1 of 3 since the human's latest answer. Implement the story: build what its criteria require, commit it, and return the result.",
    "This is attempt 2 of 3 since the human's latest answer. A worktree of an earlier attempt is in place, with its commits and what it left uncommitted: continue from it, do not start over, and fix every open finding of the story.",
    "This is attempt 3 of 3 since the human's latest answer. A worktree of an earlier attempt is in place, with its commits and what it left uncommitted: continue from it, do not start over, and fix every open finding of the story.",
  ]);
  const counts = (open) => `The story has ${open} of earlier attempts, 1 open finding that used no attempt, and 1 finding from before the human's latest answer. Fix every open one.`;
  assert.deepEqual(impls.map((c) => c.prompt.split("[attempt N] failed before the human's latest answer, and says what failed before it. ")[1].split("\n")[0]), [counts("0 open findings"), counts("1 open finding"), counts("2 open findings")]);
  for (const c of impls) {
    assert.ok(c.prompt.includes(`\nRead the story US-1 first, from OmniState, with \`OMNI_RUN=${result.run} OMNI_ROLE=impl OMNI_FEATURE='board' omni story read US-1 --json\`.`), c.prompt);
    assert.doesNotMatch(c.prompt, /pink|border|README|was blue|The badge is red/);
  }
  assert.deepEqual(findingsOf(dirs, "US-1").slice(2, 4), [["review", "[attempt 1] The badge is pink", ["src/badge.rs"], true], ["review", "[attempt 2] The badge has no border", ["src/border.rs"], true]]);
});

test("after 3 failed attempts the story goes back to the backlog, blocked, with one thread holding every attempt's findings and those before the latest answer; a story that comes with 3 failed attempts gets its thread without an implementer", { timeout: 60000 }, async (t) => {
  const open = [1, 2, 3].map((n) => ({ stage: "review", text: `[attempt ${n}] Still pink`, files: [], resolved: false }));
  const answered = { stage: "testing", text: "[attempt 1] The badge was blue", files: [], resolved: true };
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }], findings: [answered] }), story("US-2", { findings: open })]);
  const unmet = (evidence) => reviewed("fail", { unmet: { 0: evidence } });
  const roles = { "review:US-1": [unmet("Pink at src/badge.rs:1"), unmet("Still pink at src/badge.rs:1"), unmet("Pink again at src/badge.rs:2")] };

  const { result, error, calls } = await deliver(dirs, { implementers: 1 }, roles);

  assert.equal(error, null);
  assert.equal(agentsOf(calls, "impl:US-1").length, 3);
  const threads = new Map(questions(dirs).map((th) => [th.title, th]));
  const exhausted = threads.get("US-1: still failing after 3 attempts");
  assert.equal(exhausted.posts[0].body, `The story failed 3 attempts since the human's latest answer. The findings of every attempt:

Attempt 1:
- [review] Criterion not met: "The badge is red". Pink at src/badge.rs:1

Attempt 2:
- [review] Criterion not met: "The badge is red". Still pink at src/badge.rs:1

Attempt 3:
- [review] Criterion not met: "The badge is red". Pink again at src/badge.rs:2

Before the latest answer:

Attempt 1:
- [testing] The badge was blue
`);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").blocked, dirs.story("US-1").links], ["backlog", true, [{ thread_id: exhausted.id, label: "question" }]]);
  assert.deepEqual(timeline(dirs, "US-1").slice(-2), ["claim review", "move backlog"]);
  assert.equal(agentsOf(calls, "impl:US-2").length, 0);
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").blocked, threads.has("US-2: still failing after 3 attempts")], ["backlog", true, true]);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]).sort(), [["US-1", "attempts_exhausted"], ["US-2", "attempts_exhausted"]]);
});

test("no courier carries story content: however long a story's text, criteria, decisions and findings, no prompt quotes them and every courier's answer stays within 2,000 characters; every agent is told how to read the story itself, and the thread of a story out of attempts holds every finding whole", { timeout: 120000 }, async (t) => {
  const long = (label) => `${label}-LONG ${"x".repeat(3000)}`;
  const many = (attempt, n) => Array.from({ length: n }, (_, i) => ({ stage: "review", text: `[attempt ${attempt}] ${long(`a${attempt}-${i + 1}`)}`, files: [], resolved: false }));
  const fields = (findings) => ({
    text: long("text"),
    criteria: [{ text: long("criterion"), human_check: false }, { text: "A regression test covers the badge", human_check: false }],
    decisions: Array.from({ length: 10 }, (_, i) => ({ question: long(`question-${i}`), choice: long(`choice-${i}`), by: "human" })),
    findings,
  });
  const dirs = scratch(t, [story("US-1", fields([...many(1, 8), ...many(2, 8)])), story("US-2", fields([...many(1, 5), ...many(2, 5), ...many(3, 5)]))]);

  const { result, error, calls } = await deliver(dirs, { implementers: 1 });

  assert.equal(error, null);
  const roles = ["clarity", "impl", "review", "verify", "tests"];
  for (const role of roles) {
    const [c] = agentsOf(calls, `${role}:US-1`);
    assert.ok(c.prompt.includes(`\`OMNI_RUN=${result.run} OMNI_ROLE=${role} OMNI_FEATURE='board' omni story read US-1 --json\``), `${role}: ${c.prompt}`);
    assert.doesNotMatch(c.prompt, /LONG|regression test covers/, role);
  }
  const outputs = couriersOf(calls).map((c) => [c.label, String(c.output || "").length]);
  assert.deepEqual(outputs.filter(([, n]) => n > 2000), [], JSON.stringify([...outputs].sort((a, b) => b[1] - a[1]).slice(0, 3)));
  assert.doesNotMatch(couriersOf(calls).map((c) => c.output).join("\n"), /LONG/);
  const [exhausted] = questions(dirs).filter((th) => th.title === "US-2: still failing after 3 attempts");
  for (const f of dirs.story("US-2").findings) assert.ok(exhausted.posts[0].body.includes(f.text.replace(/^\[attempt \d\] /, "")), f.text.slice(0, 40));
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]).sort(), [["US-1", "done"], ["US-2", "attempts_exhausted"]]);
});

// Every string in a payload, however deep.
const stringsIn = (v) => (typeof v === "string" ? [v] : Array.isArray(v) ? v.flatMap(stringsIn) : v && typeof v === "object" ? Object.values(v).flatMap(stringsIn) : []);

test("no payload carries an agent's long text: whatever clarity, the council, the implementer, the reviewer, the verifier, the test author and the journeys check return, no string a courier copies passes 500 characters, and the board gets their text whole, from the files the agents wrote", { timeout: 300000 }, async (t) => {
  const long = (label) => `${label}-LONG "quoted" \\ ${"x".repeat(3000)}`;
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the badge", human_check: false }];
  const dirs = scratch(t, ["US-1", "US-2", "US-3", "US-4", "US-5"].map((key) => story(key, { criteria })));
  const asked = (label, impact, fields = {}) => gap(long(label), impact, { criterion: long(`${label}-criterion`), options: [long(`${label}-a`), long(`${label}-b`)], recommendation: long(`${label}-rec`), reason: long(`${label}-reason`), ...fields });
  // A member settles one gap with a long quote, splits on another, where the
  // user's member gives no quote, and has no pick for the third.
  const member = (reading) => (call) => {
    const q = questionIn(call.prompt);
    if (q.startsWith("settled")) return vote(1, { quote: long(`settled-${reading}-quote`), source: long("settled-source") });
    if (q.startsWith("split")) return vote(reading === "code" ? 2 : 1, { quote: reading === "user" ? "" : long(`split-${reading}-quote`), source: reading === "user" ? "" : "doc d1 node n1", why: long(`split-${reading}-why`) });
    if (q.startsWith("impl")) return vote(1, { quote: long(`impl-${reading}-quote`), source: "src/badge.rs:1" });
    return vote(0);
  };
  const council = (key) => councilOf(key, ...["spec", "code", "user"].map((reading) => Array.from({ length: 3 }, () => member(reading))));
  const roles = {
    "clarity:US-1": [{ testObligations: [1], gaps: [asked("minor", "ux", { blocking: false }), asked("settled", "scope"), asked("split", "scope"), asked("third", "ux"), asked("deferred", "technical")] }],
    ...council("US-1"),
    ...council("US-2"),
    "impl:US-2": [
      { status: "needs_decision", summary: "", question: long("impl-q"), options: [long("impl-a"), long("impl-b")], recommendation: long("impl-rec") },
      built("US-2.txt", { decisions: [{ question: long("decision-q"), choice: long("decision-c") }] }),
    ],
    "review:US-2": [reviewed("fail", { unmet: { 0: long("evidence") }, defects: [{ text: long("defect"), files: ["src/badge.rs"], criterion: long("defect-criterion") }] })],
    "verify:US-2": [verified({ 0: long("observed") })],
    "tests:US-2": [authored({ status: "found_defect", defect: { test: long("test"), observed: long("test-observed"), expected: long("test-expected") } })],
    "impl:US-3": [{ status: "blocked", summary: long("summary") }],
    "tests:US-4": [authored({ status: "blocked", blockers: long("blockers") })],
    final: [{ healthy: true, journeys: [long("journey")], report: long("report") }],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const longest = couriersOf(calls).filter((c) => fileIn(c.prompt) !== null).map((c) => [c.label, Math.max(0, ...stringsIn(payloadOf(c)).map((x) => x.length))]);
  assert.deepEqual(longest.filter(([, n]) => n > 500), [], JSON.stringify([...longest].sort((a, b) => b[1] - a[1]).slice(0, 3)));
  assert.doesNotMatch(couriersOf(calls).map((c) => payloadIn(c.prompt) || "").join("\n"), /LONG/);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]).sort(), [["US-2", "attempts_exhausted"], ["US-3", "blocked"], ["US-4", "blocked"], ["US-5", "done"]]);
  const board = JSON.stringify(dirs.state().stories) + JSON.stringify(dirs.state().threads);
  for (const label of [
    "minor", "minor-rec", "deferred", "deferred-rec", "settled", "settled-a", "settled-spec-quote", "settled-source", "split", "split-criterion", "split-a", "split-b", "split-code-quote", "split-user-why", "split-reason", "third",
    "impl-q", "impl-a", "impl-user-quote", "decision-q", "decision-c", "evidence", "defect", "defect-criterion", "observed", "test", "test-observed", "test-expected",
    "summary", "blockers", "journey", "report",
  ]) assert.ok(board.includes(JSON.stringify(long(label)).slice(1, -1)), label);
});

test("the test author ignores the story's findings: its oracle comes from the story's text, criteria, decisions and linked doc nodes only; the implementer and the reviewer still read them", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }], findings: [{ stage: "testing", text: "rebase conflict in README", files: ["README"], resolved: false }] })]);

  const { error, calls } = await deliver(dirs);

  assert.equal(error, null);
  const [tests] = agentsOf(calls, "tests:US-1");
  assert.ok(tests.prompt.includes("\n- findings: ignore them, though the JSON holds them. They say what earlier attempts got wrong, not what the story requires: your oracle comes from the story's text, its criteria, its decisions and its linked doc nodes only.\n"), tests.prompt);
  for (const role of ["impl", "review"]) {
    const [c] = agentsOf(calls, `${role}:US-1`);
    assert.match(c.prompt, /\n- findings: each with its stage, text, files and resolved\./, role);
    assert.doesNotMatch(c.prompt, /ignore them/, role);
  }
});

test("clarity names a test obligation by the criterion's number, and the helper writes its decision from the criterion's text on the board, so that no copy can miss it: the review is told it is an obligation; a number past the story's criteria fails the check, and the story is not delivered in this run", { timeout: 120000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the \"red\" badge in `src/badge.rs`", human_check: false }];
  const dirs = scratch(t, [story("US-1", { criteria }), story("US-2", { criteria })]);
  const roles = { "clarity:US-1": [{ testObligations: [1], gaps: [] }], "clarity:US-2": [{ testObligations: [2], gaps: [] }] };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: `Is "${criteria[1].text}" behaviour or a test?`, choice: "treated as a test obligation: the lane's test author owns it, and the verifier skips it", by: "agent" }]);
  assert.equal(owedIn(agentsOf(calls, "review:US-1")[0].prompt), "Criterion 1 is a test obligation.");
  assert.deepEqual(dirs.story("US-2").decisions, []);
  assert.equal(agentsOf(calls, "courier:obligations:US-2").length, 1);
  assert.ok(logs.some((l) => l.startsWith("US-2: the clarity check failed: ") && l.includes("US-2 has 2 criteria, numbered from 0, so 2 names none of them")), logs.join("\n"));
  assert.deepEqual([agentsOf(calls, "impl:US-2"), result.outcomes.map((o) => [o.key, o.outcome])], [[], [["US-1", "done"]]]);
});

test("a journeys check that writes no result file keeps neither its journeys off the report nor the lock: short, they go in the finish payload, which the helper takes", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const roles = { final: [{ healthy: true, journeys: ["Badge: works"], report: "Fine." }] };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles, { written: (call, r) => (call.label === "final" ? null : JSON.stringify(r)) });

  assert.equal(error, null);
  assert.deepEqual([agentsOf(calls, "courier:finish").length, agentsOf(calls, "again:final").length], [2, 0]);
  const posted = dirs.state().threads.find((th) => th.id === result.thread).posts;
  assert.deepEqual([posted.length, result.posted, existsSync(join(dirs.repo, ".omni", "run.lock"))], [1, true, false]);
  assert.ok(posted[0].body.includes("\n## Journeys\n- healthy: Fine.\n- Badge: works\n"), posted[0].body);
  assert.ok(logs.some((l) => l.startsWith("courier:finish: ") && l.includes("cannot be read as its agent returned it; its 17 characters go in the payload")), logs.join("\n"));
});

test("an agent that writes no result file costs no story: an implementer's question and decision, and a council member's vote, reach the board from the payload when short, from the file the same role writes once more when long, and cut to 500 characters with a note when that file is wrong too; no story is blocked or stops on an error", { timeout: 300000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  // US-1's texts are short; US-2's and US-3's long.
  const text = (key, what) => (key === "US-1" ? `${what}?` : `${key}-${what}-LONG ${"x".repeat(1000)}`);
  const roles = Object.fromEntries(keys.flatMap((key) => [
    [`impl:${key}`, [
      { status: "needs_decision", summary: "Stopped.", question: text(key, "q"), options: ["Red", "Blue"], recommendation: "Red" },
      built(`${key}.txt`, { decisions: [{ question: text(key, "decision"), choice: "None" }] }),
    ]],
    ...Object.entries(councilOf(key, [vote(1, { quote: text(key, "quote"), source: "doc d1 node n1" })], [vote(1, { quote: "red", source: "src/a.rs:1" })], [vote(1, { quote: "red", source: "src/b.rs:1" })])),
  ]));
  // The implementers and the spec members write no file; asked again, US-3's write none either.
  const written = (call, r) => {
    const label = call.label.replace(/^again:/, "");
    const slips = label.startsWith("impl:") || label.endsWith(":spec");
    return slips && (!call.label.startsWith("again:") || label.includes("US-3")) ? null : JSON.stringify(r);
  };

  const { result, error, calls } = await deliver(dirs, {}, roles, { written });

  assert.equal(error, null);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]).sort(), keys.map((key) => [key, "done"]));
  assert.deepEqual([questions(dirs), keys.map((key) => dirs.story(key).blocked), result.halted], [[], [false, false, false], null]);
  const choice = (quote) => `Red (council: "${quote}" (doc d1 node n1); "red" (src/a.rs:1); "red" (src/b.rs:1))`;
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "q?", choice: choice("quote?"), by: "agent" }, { question: "decision?", choice: "None", by: "agent" }]);
  assert.deepEqual(dirs.story("US-2").decisions, [{ question: text("US-2", "q"), choice: choice(text("US-2", "quote")), by: "agent" }, { question: text("US-2", "decision"), choice: "None", by: "agent" }]);
  // Cut, a part shares the 500 characters with the other parts of its file: "Red", "None" and the source.
  const cut = (s, n) => `${s.slice(0, n).trimEnd()}… [cut: the agent's full result could not be carried]`;
  assert.deepEqual(dirs.story("US-3").decisions, [{ question: cut(text("US-3", "q"), 497), choice: choice(cut(text("US-3", "quote"), 486)), by: "agent" }, { question: cut(text("US-3", "decision"), 496), choice: "None", by: "agent" }]);
  const again = calls.filter((c) => c.label.startsWith("again:"));
  assert.deepEqual(again.map((c) => c.label).sort(), ["again:council:US-2:spec", "again:council:US-3:spec", "again:impl:US-2", "again:impl:US-2", "again:impl:US-3", "again:impl:US-3"]);
  assert.deepEqual([...new Set(again.map((c) => c.model))].sort(), ["opus", "sonnet"]);
});

test("an error over an agent's result file never counts toward the stop on a repeated error, nor blocks its story: two stories that stop on one, with errors alike once their keys are taken out, are only released", { timeout: 120000 }, async (t) => {
  const keys = ["US-1", "US-2"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  // Without the fallback, a refused result file reaches the breaker.
  const carriedOn = "const next = e && e.outFile ? await recover(sent, e.outFile, courier, past) : null";
  assert.ok(SOURCE.includes(carriedOn));
  const roles = Object.fromEntries(keys.map((key) => [`impl:${key}`, [built(`${key}.txt`, { decisions: [{ question: "Which red?", choice: "#c00" }] })]]));

  const { result, error, logs } = await deliver(dirs, {}, roles, { source: SOURCE.replace(carriedOn, "const next = null"), written: (call, r) => (call.label.startsWith("impl:") ? null : JSON.stringify(r)) });

  assert.equal(error, null);
  assert.deepEqual([result.halted, questions(dirs)], [null, []]);
  assert.deepEqual(keys.map((key) => [dirs.story(key).status, dirs.story(key).holder, dirs.story(key).blocked]), [["backlog", null, false], ["backlog", null, false]]);
  assert.deepEqual(result.outcomes.map((o) => o.outcome), ["failed", "failed"]);
  assert.ok(logs.some((l) => l.startsWith("US-1: stopped on an agent's result file, and released without a thread: ")), logs.join("\n"));
});

test("the runtime may hand the script a copy of what an agent returned, and parallel of what its thunks gave: a council member's quote still reaches the board whole", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  const roles = {
    "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }],
    ...councilOf("US-1", [vote(1, { quote: "Badges are red", source: "doc d1 node n1" })], [vote(1)], [vote(1)]),
  };

  const { error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which colour is the badge?", choice: 'Red (council: "Badges are red" (doc d1 node n1))', by: "agent" }]);
});

test("a part of an agent's result whose file the script does not know fails loudly, and nothing is written in its place", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked_by: ["US-9"] }), blocker()]);
  const marked = "return r && typeof r === 'object' && !Array.isArray(r) ? { ...r, [WRITTEN]: { file, role, model, label: opts.label } } : r";
  assert.ok(SOURCE.includes(marked));
  const roles = { "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux", { blocking: false })] }] };

  const { error, logs } = await deliver(dirs, {}, roles, { source: SOURCE.replace(marked, "return r") });

  assert.equal(error, null);
  assert.ok(logs.some((l) => l.startsWith("US-1: the clarity check failed: ") && l.includes("with no result file on it")), logs.join("\n"));
  assert.deepEqual(dirs.story("US-1").decisions, []);
});

test("a check's finding quotes the end of the check's log, which the helper reads by the job's id: no courier carries the output, and the next implementer reads the finding from the board", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const count = join(dirs.root, "count");
  // Fails once, on the story's first lane pass, with a log whose last line names the error; passes after.
  const fast = `n=$(cat ${count} 2>/dev/null || echo 0); echo $((n+1)) > ${count}; if [ "$n" = 0 ]; then seq 1 600; echo "FINAL-$((6*7))-ERROR at src/badge.rs"; exit 1; fi; true`;

  const { result, error, calls } = await deliver(dirs, { implementers: 1, checks: { fast } });

  assert.equal(error, null);
  const [finding] = dirs.story("US-1").findings.map((f) => f.text);
  assert.ok(finding.startsWith(`[attempt 1] The fast check (\`${fast}\`) failed. It passes on deliver/board without the story.\n\nThe end of its output:\n\`\`\`\n`), finding);
  assert.ok(finding.endsWith("\n599\n600\nFINAL-42-ERROR at src/badge.rs\n```"), finding);
  assert.doesNotMatch(couriersOf(calls).map((c) => c.output).join("\n"), /FINAL-42-ERROR/);
  const [, impl] = agentsOf(calls, "impl:US-1");
  assert.doesNotMatch(impl.prompt, /FINAL/);
  assert.ok(impl.prompt.includes("The story has 1 open finding of earlier attempts"), impl.prompt);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("an implementer's question the council settles becomes an agent decision and the implementer runs again with it; a reviewer's question it cannot settle opens a thread and blocks the story", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const asked = { status: "needs_decision", summary: "Stopped.", question: "Which red?", options: ["#c00", "#f00"], recommendation: "#c00" };
  const blink = { verdict: "needs_decision", criteria: [], defects: [], question: "Should the badge blink?", options: ["Yes", "No"], recommendation: "No" };
  const quoted = vote(1, { quote: "const RED = '#c00'", source: "src/colors.rs:3" });
  const roles = {
    "impl:US-1": [asked, built("src/badge.rs")],
    "review:US-1": [blink],
    "council:US-1:spec": [vote(1), vote(2)],
    "council:US-1:code": [quoted, vote(0)],
    "council:US-1:user": [vote(1), vote(2)],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const impls = agentsOf(calls, "impl:US-1");
  assert.equal(impls.length, 2);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Which red?", choice: "#c00 (council: \"const RED = '#c00'\" (src/colors.rs:3))", by: "agent" }]);
  assert.match(lineAfter(impls[1].prompt, "## Your task now\n"), /An earlier session of this attempt stopped to ask a question, which the story's decisions now settle\./);
  for (const c of agentsOf(calls, "council:US-1:code")) assert.ok(c.prompt.includes(`You read the code, and nothing else: the story's worktree ${dirs.worktree("US-1")}.`), c.prompt);
  assert.equal(agentsOf(calls, "council:US-1:code").length, 2);
  // Every member reads the story itself; only the spec member reads its linked docs.
  for (const reading of ["spec", "code", "user"]) {
    const [c] = agentsOf(calls, `council:US-1:${reading}`);
    assert.ok(c.prompt.includes(`\`OMNI_RUN=${result.run} OMNI_ROLE=council OMNI_FEATURE='board' omni story read US-1 --json\``), c.prompt);
    assert.equal(c.prompt.includes("doc read <doc_id>"), reading === "spec", reading);
    assert.doesNotMatch(c.prompt, /The badge is red/);
  }
  const [blinking] = questions(dirs);
  assert.deepEqual([questions(dirs).length, blinking.title], [1, "US-1: Should the badge blink?"]);
  assert.match(blinking.posts[0].body, /\nAsked by the reviewer\.\n[\s\S]*\nRecommendation: No \(the council's majority\)\n$/);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").blocked], ["implemented", true]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "needs_decision", thread: blinking.id }]);
  assert.deepEqual(result.council, { settled: 1, majority: 0, asked: 1 });
});

test("an implementer that asks a fourth question in one attempt, after the council settled three, has it sent to the human without a council", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const ask = (n) => ({ status: "needs_decision", summary: "Stopped.", question: `Question ${n}?`, options: ["A", "B"], recommendation: "B" });
  const quoted = vote(1, { quote: "Use A", source: "src/a.rs:1" });
  const roles = { "impl:US-1": [1, 2, 3, 4].map(ask), ...councilOf("US-1", [quoted, quoted, quoted], [vote(1), vote(1), vote(1)], [vote(1), vote(1), vote(1)]) };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.equal(calls.filter((c) => c.label.startsWith("council:")).length, 9);
  assert.deepEqual(dirs.story("US-1").decisions.map((d) => d.question), ["Question 1?", "Question 2?", "Question 3?"]);
  const [fourth] = questions(dirs);
  assert.equal(fourth.title, "US-1: Question 4?");
  assert.match(fourth.posts[0].body, /\nThe council: not asked: it settled 3 questions of the implementer in this attempt already\n\nRecommendation: B \(the recommendation of the implementer\)\n$/);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").blocked], ["backlog", true]);
  assert.deepEqual(result.council, { settled: 3, majority: 0, asked: 1 });
});

test("an implementer's slot is free once its story is implemented: with implementers 3, a fourth story starts implementing while an earlier one waits in review", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  let fourthStarted;
  const fourth = new Promise((resolve) => { fourthStarted = resolve; });
  // The first three stories' reviewers answer once the fourth story's
  // implementer has started, or after 5 s if it never does: with their slots
  // held through review, it would not start before one of them ended.
  const waiting = async (call, serial, board) => {
    await within(fourth, 5000);
    return reviewed("pass")(call, serial, board);
  };
  const roles = {
    ...Object.fromEntries(["US-1", "US-2", "US-3"].map((key) => [`review:${key}`, [waiting]])),
    "impl:US-4": [(call, serial) => {
      fourthStarted();
      return built("US-4.txt")(call, serial);
    }],
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const order = boardLog(dirs);
  const fourthClaim = order.indexOf("claim US-4 implementing");
  const earlier = ["US-1", "US-2", "US-3"];
  assert.ok(fourthClaim !== -1, order.join("\n"));
  // It started before any of the first three left review, while one was in review already.
  for (const key of earlier) assert.ok(fourthClaim < order.indexOf(`move ${key} reviewed`), order.join("\n"));
  assert.ok(earlier.some((key) => order.indexOf(`claim ${key} review`) < fourthClaim), order.join("\n"));
  assert.deepEqual(keys.map((key) => dirs.story(key).status), ["done", "done", "done", "done"]);
  assert.deepEqual(result.outcomes.map((o) => o.key).sort(), keys);
});

test("at most `implementers` stories implement at once and at most `reviewers` are in review", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  // A review takes half a second, long enough for the next story's to start beside it.
  const slow = async (call, serial, board) => {
    await pause(500);
    return reviewed("pass")(call, serial, board);
  };

  const { result, error } = await deliver(dirs, { implementers: 2, reviewers: 1 }, Object.fromEntries(keys.map((key) => [`review:${key}`, [slow]])));

  assert.equal(error, null);
  assert.deepEqual([peakIn(dirs, "implementing", ["implemented"]), peakIn(dirs, "review", ["reviewed", "backlog"])], [2, 1]);
  assert.deepEqual(keys.map((key) => dirs.story(key).status), ["done", "done", "done", "done"]);
  assert.deepEqual(result.outcomes.map((o) => o.key).sort(), keys);
});

test("a story its review sends back stays in review, its files busy, until an implementer slot is free, takes that slot ahead of new claims, and only then goes back to the backlog and is claimed again, back to back: a story planned on its files is claimed only once it is done", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [
    story("US-1", { priority: 1, files_planned: ["src/badge.rs"] }),
    story("US-2", { files_planned: ["src/a.rs"] }),
    story("US-3", { files_planned: ["src/b.rs"] }),
    story("US-5", { priority: 2, files_planned: ["src/badge.rs"] }),
  ]);
  let meanwhile = null;
  const roles = {
    "impl:US-1": [built("src/badge.rs"), built("src/badge.rs")],
    "review:US-1": [reviewed("fail", { defects: [{ text: "The badge is pink", files: ["src/badge.rs"], criterion: "" }] }), reviewed("pass")],
    // US-2 holds the one slot until US-1 has its finding, and a second more
    // for US-1 to go back to the backlog, which it does not while it waits.
    "impl:US-2": [async (call, serial) => {
      for (let i = 0; i < 400 && !(await serial(() => (dirs.story("US-1").findings || []).length > 0)); i++) await pause(50);
      for (let i = 0; i < 20 && (await serial(() => dirs.story("US-1").status)) !== "backlog"; i++) await pause(50);
      meanwhile = await serial(() => dirs.story("US-1").status);
      return built("src/a.rs")(call, serial);
    }],
  };

  const { error } = await deliver(dirs, { implementers: 1 }, roles);

  assert.equal(error, null);
  const order = boardLog(dirs);
  assert.equal(meanwhile, "review");
  assert.deepEqual(order.filter((e) => e.startsWith("claim US-1 ") || e.startsWith("move US-1 ")), [
    "claim US-1 implementing", "move US-1 implemented", "claim US-1 review", "move US-1 backlog",
    "claim US-1 implementing", "move US-1 implemented", "claim US-1 review", "move US-1 reviewed",
    "claim US-1 testing", "move US-1 done",
  ]);
  const back = order.indexOf("move US-1 backlog");
  assert.ok(order.indexOf("move US-2 implemented") < back, order.join("\n"));
  assert.equal(order.slice(back + 1).find((e) => e.endsWith(" implementing")), "claim US-1 implementing", order.join("\n"));
  assert.ok(order.indexOf("claim US-3 implementing") > order.lastIndexOf("claim US-1 implementing"), order.join("\n"));
  assert.ok(order.indexOf("claim US-5 implementing") > order.indexOf("move US-1 done"), order.join("\n"));
  assert.equal(peakIn(dirs, "implementing", ["implemented"]), 1);
});

test("the deliver loop lists the board for the next story only once its last claim is made: of two stories planned on one file, with two implementer slots, the second is claimed once the first is done, whether that claim is slow or quick", { timeout: 120000 }, async (t) => {
  for (const slow of [true, false]) {
    const dirs = scratch(t, [story("US-1", { files_planned: ["shared.rs"] }), story("US-2", { files_planned: ["shared.rs"] })]);
    // US-1's claims take 2 s on the board, as a slow courier would leave them.
    if (slow) Object.assign(dirs.env, { FAKE_OMNI_HANG: "story claim US-1", FAKE_OMNI_HANG_MS: "2000" });

    const { result, error } = await deliver(dirs, { implementers: 2 });

    assert.equal(error, null);
    const order = boardLog(dirs);
    assert.ok(order.indexOf("claim US-2 implementing") > order.indexOf("move US-1 done"), `${slow ? "slow" : "quick"} claim:\n${order.join("\n")}`);
    assert.equal(peakIn(dirs, "implementing", ["implemented", "backlog", "done"]), 1);
    assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }, { key: "US-2", outcome: "done" }]);
  }
});

// The script with its agent cap or its job wait set lower; the real values are pinned first.
const capped = (values) => {
  let source = SOURCE;
  for (const [name, value] of Object.entries(values)) {
    const real = { AGENT_CAP: 900, HARD_CAP: 960, STORY_CALLS: 90, CHECK_CALLS: 20, JOB_WAIT: 480 }[name];
    assert.ok(source.includes(`\nconst ${name} = ${real}\n`), `${name} is ${real}`);
    source = source.replace(`\nconst ${name} = ${real}\n`, `\nconst ${name} = ${value}\n`);
  }
  return source;
};

test("the run counts every agent call, couriers too, and takes a new story only while the calls made and those set aside for the stories in flight, 90 each with the new one, stay within 900", { timeout: 60000 }, async (t) => {
  // 12 calls come before the first claim. With 60 and 15 for 900 and 60, the
  // first two stories fit; the third would be the third in flight, past 60.
  const source = capped({ AGENT_CAP: 60, STORY_CALLS: 15 });
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);

  const { result, error, logs } = await deliver(dirs, { implementers: 2 }, {}, { source });

  assert.equal(error, null);
  assert.deepEqual(["US-1", "US-2", "US-3"].map((key) => claimsOf(dirs, key)), [["implementing", "review", "testing"], ["implementing", "review", "testing"], []]);
  assert.deepEqual(["US-1", "US-2", "US-3"].map((key) => dirs.story(key).status), ["done", "done", "backlog"]);
  assert.equal(result.capped, true);
  assert.ok(logs.some((l) => /^The run has made \d+ agent calls, with 2 stories in flight: it takes no new story, and ends once those are done; the next run carries on$/.test(l)), logs.join("\n"));
});

test("a story in flight has 90 agent calls set aside: one whose lane fails three times makes more than 60 and at most 90, couriers included", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const pink = verified({ 0: "It is pink" });

  const { result, error, calls } = await deliver(dirs, {}, { "verify:US-1": [pink, pink, pink] });

  assert.equal(error, null);
  assert.deepEqual(result.outcomes.map((o) => o.outcome), ["attempts_exhausted"]);
  const made = calls.filter((c) => c.label.endsWith(":US-1") || c.label.includes(":US-1:")).length;
  assert.ok(made > 60 && made <= 90, `${made} calls`);
  assert.ok(SOURCE.includes("\nconst STORY_CALLS = 90\n"), "STORY_CALLS is 90");
});

test("a story in flight that reaches 960 agent calls stops there: it is released with no attempt used, the run says so, and the calls stay under the runtime's 1000", { timeout: 60000 }, async (t) => {
  // The implementer asks three questions the council settles, which keeps
  // the story claimed from its 8th call to about its 32nd; it stops at 20.
  const source = capped({ AGENT_CAP: 15, HARD_CAP: 20, STORY_CALLS: 1, CHECK_CALLS: 1 });
  const dirs = scratch(t, [story("US-1")]);
  const asks = (n) => ({ status: "needs_decision", summary: "Stopped.", question: `Question ${n}?`, options: ["A", "B"], recommendation: "B" });
  const quoted = vote(1, { quote: "Use A", source: "src/a.rs:1" });
  const roles = { "impl:US-1": [asks(1), asks(2), asks(3), built("src/badge.rs")], ...councilOf("US-1", [quoted, quoted, quoted], [vote(1), vote(1), vote(1)], [vote(1), vote(1), vote(1)]) };

  const { result, error, logs } = await deliver(dirs, {}, roles, { source });

  assert.equal(error, null);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "stopped", detail: "stopped at the agent cap of 20" }]);
  assert.ok(logs.includes("US-1: stopped at 20 agent calls, and released without using an attempt; the next run carries on"), logs.join("\n"));
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.blocked, done.findings || [], questions(dirs)], ["backlog", null, false, [], []]);
  assert.ok(result.agents <= 20 + 4, `${result.agents} calls`);
  assert.equal(result.capped, true);
});

test("clarity starts a check only while the calls made and those set aside for the checks in flight stay within the cap; a check raises no gap a decision settles, and records its story after its own writes", { timeout: 60000 }, async (t) => {
  // 3 calls come before clarity: the first three checks fit 20 at 5 each, the rest wait for the next run.
  const source = capped({ AGENT_CAP: 20, CHECK_CALLS: 5 });
  const keys = ["US-1", "US-2", "US-3", "US-4", "US-5"];
  const decided = { question: "Which colour is the badge?", choice: "Red", by: "human" };
  const dirs = scratch(t, keys.map((key) => story(key, key === "US-1" ? { decisions: [decided] } : {})));
  const roles = { "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux"), gap("Which shade?", "ux", { blocking: false, recommendation: "Crimson" })] }] };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles, { source });

  assert.equal(error, null);
  assert.deepEqual(keys.map((key) => agentsOf(calls, `clarity:${key}`).length), [1, 1, 1, 0, 0]);
  for (const key of ["US-4", "US-5"]) assert.ok(logs.some((l) => l.startsWith(`${key}: not checked: the run has made `)), logs.join("\n"));
  assert.equal(calls.filter((c) => c.label.startsWith("council:")).length, 0);
  assert.deepEqual(dirs.story("US-1").decisions, [decided, { question: "Which shade?", choice: "Crimson", by: "agent" }]);
  assert.deepEqual((await helperStep(dirs, await nextRun(dirs), "clarity-pending")).keys, ["US-4", "US-5"]);
  assert.deepEqual([result.capped, keys.flatMap((key) => claimsOf(dirs, key))], [true, []]);
});

test("prompts give the context first and the criteria and the task last, tell the agent to read the story, its decisions and its linked doc nodes with omni as this run's role and quote none of them, use absolute paths, say who is read-only and that the workflow makes every board write; models and agent types come from args", { timeout: 60000 }, async (t) => {
  const docs = { "doc-spec": '<p data-omni-id="n1">SPEC NODE ONE</p><p data-omni-id="n2">SPEC NODE TWO</p>' };
  const dirs = scratch(t, [story("US-1", {
    text: "Show a badge.",
    criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }],
    decisions: [{ question: "Which red?", choice: "#c00", by: "human" }],
    links: [{ doc_id: "doc-spec", node_id: "n1", label: "spec" }, { doc_id: "doc-spec", node_id: "n2", label: "counts" }],
  })], [], { docs });

  const { result, error, calls } = await deliver(dirs, { models: { review: "sonnet" }, agentTypes: { impl: "implementer", review: "verifier" }, checks: { stack: "make stack-tests" } });

  assert.equal(error, null);
  const [clarity] = agentsOf(calls, "clarity:US-1");
  const [impl] = agentsOf(calls, "impl:US-1");
  const [review] = agentsOf(calls, "review:US-1");
  assert.deepEqual([clarity, impl, review].map((c) => [c.model, c.agentType]), [["opus", "omni-clarity"], ["sonnet", "implementer"], ["sonnet", "verifier"]]);
  assert.equal(result.checks.stack, "make stack-tests");
  const order = (prompt, headings) => headings.map((h) => prompt.indexOf(`\n${h}\n`));
  for (const [prompt, headings] of [
    [clarity.prompt, ["## The story", "## The shared parts", "## The board", "## Acceptance criteria", "## Your result, as a file", "## Your task now"]],
    [impl.prompt, ["## Your worktree", "## The story", "## The board", "## Rules", "## Acceptance criteria", "## Your result, as a file", "## Your task now"]],
    [review.prompt, ["## The change", "## The story", "## The board", "## Rules", "## Acceptance criteria", "## Your result, as a file", "## Your task now"]],
  ]) {
    const at = order(prompt, headings);
    assert.ok(at.every((n) => n > 0), `${headings} in ${prompt}`);
    assert.deepEqual(at, [...at].sort((a, b) => a - b), prompt);
    assert.match(prompt, /\nThe workflow makes every write to the board itself, through its helper, from what you return\./);
    assert.doesNotMatch(prompt, /SPEC NODE|Show a badge|The badge is red|It looks right|Which red|#c00|doc-spec|n1|counts/);
  }
  for (const [prompt, role] of [[clarity.prompt, "clarity"], [impl.prompt, "impl"], [review.prompt, "review"]]) {
    assert.equal(resultFileIn(prompt), join(dirs.repo, ".omni", "runs", result.run, "out", `${role}-US-1-1.json`));
    assert.ok(prompt.includes(`\n## The story\nRead the story US-1 first, from OmniState, with \`OMNI_RUN=${result.run} OMNI_ROLE=${role} OMNI_FEATURE='board' omni story read US-1 --json\`. It prints the story as JSON:\n`), prompt);
    assert.ok(prompt.includes(`\n- links: each one with a doc_id and a node_id is a node of a doc the story links. Read each of those docs once, with \`OMNI_RUN=${result.run} OMNI_ROLE=${role} OMNI_FEATURE='board' omni doc read <doc_id>\`, and find in it each linked node: the element whose data-omni-id is the node_id.\n`), prompt);
  }
  assert.ok(clarity.prompt.includes(`\`OMNI_RUN=${result.run} OMNI_ROLE=clarity OMNI_FEATURE='board' omni doc list\``));
  assert.match(clarity.prompt, /\n- decisions: each a question, the choice taken, and by whom, agent or human\. They are settled: a gap one of them already settles is NOT raised again\.\n/);
  assert.match(clarity.prompt, /\n## Acceptance criteria\nThe story's 2 criteria, numbered from 0 to 1 in the order the story gives them\. A criterion with human_check true is confirmed by a human later\.\n/);
  assert.match(impl.prompt, /\n- decisions: each a question, the choice taken, and by whom, agent or human\. They are settled: build against them as if the spec said so\.\n/);
  assert.match(impl.prompt, /Never start the stack or run the stack tests \(`make stack-tests`\)/);
  assert.ok(review.prompt.includes(`See the diff with \`git -C ${dirs.worktree("US-1")} diff refs/heads/deliver/board...HEAD\``));
  assert.match(review.prompt, /\n## Acceptance criteria\nThe story's 2 criteria, numbered from 0 to 1 in the order the story gives them\. No criterion is a test obligation\.\n/);
  for (const prompt of [clarity.prompt, review.prompt]) assert.match(prompt, /You are read-only: you change no file but your result file, make no commit, and write nothing to the board\./);
  assert.doesNotMatch(impl.prompt, /You are read-only/);
});

// ---- the agent types ------------------------------------------------------------------

// One story through every role: a clarity gap the council settles, a review
// that fails once, so that attempt 2 runs on implRetry, the lane's verifier
// and test author, and the final check.
async function everyRole(t, agentTypes) {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge has a colour", human_check: false }] })]);
  const quoted = vote(1, { quote: "Badges are red", source: "doc doc-spec node n1" });
  const roles = {
    "clarity:US-1": [{ testObligations: [], gaps: [gap("Which colour is the badge?", "ux")] }],
    ...councilOf("US-1", [quoted], [vote(1)], [vote(1)]),
    "review:US-1": [reviewed("fail", { unmet: { 0: "The badge has no colour" } }), reviewed("pass")],
  };
  const run = await deliver(dirs, agentTypes ? { agentTypes } : {}, roles);
  assert.equal(run.error, null);
  assert.equal(dirs.story("US-1").status, "done");
  // Each call by role: the label's first part, and implRetry for the
  // implementer's second attempt.
  const byRole = {};
  for (const c of run.calls) {
    let role = c.label.split(":")[0];
    if (role === "impl" && byRole.impl) role = "implRetry";
    (byRole[role] ||= []).push(c);
  }
  return byRole;
}
const ROLES = ["clarity", "council", "impl", "implRetry", "review", "verify", "tests", "final", "courier"];
const typeOf = (role) => (role === "implRetry" ? "omni-impl" : `omni-${role}`);

test("every role runs as its omni- agent type by default, omni-impl for implRetry too, and the courier as omni-courier", { timeout: 60000 }, async (t) => {
  const byRole = await everyRole(t);

  assert.deepEqual(Object.keys(byRole).sort(), [...ROLES].sort());
  for (const role of ROLES) assert.deepEqual([...new Set(byRole[role].map((c) => c.agentType))], [typeOf(role)], role);
});

test("args.agentTypes overrides the agent type of the roles it names, implRetry apart from impl, and leaves the others at their default", { timeout: 60000 }, async (t) => {
  const byRole = await everyRole(t, { implRetry: "implementer", courier: "carrier" });

  const expected = { ...Object.fromEntries(ROLES.map((role) => [role, typeOf(role)])), implRetry: "implementer", courier: "carrier" };
  for (const role of ROLES) assert.deepEqual([...new Set(byRole[role].map((c) => c.agentType))], [expected[role]], role);
});

test("every agent file parses, is named after its file, and lists the tools its role's prompts ask for: Write and Read for a result or payload file, Bash for a command, Edit only where the prompt is not read-only, a browser where it says to use one", { timeout: 60000 }, async (t) => {
  const agents = readAgents();
  for (const a of agents) {
    assert.equal(a.file, `${a.name}.md`);
    assert.ok(a.description && !a.description.includes(": "), `${a.file}: a description with no ": ", which YAML would read as a mapping`);
    assert.ok(a.tools.length && a.body, a.file);
  }
  const byName = new Map(agents.map((a) => [a.name, a]));
  const byRole = await everyRole(t);

  assert.deepEqual([...byName.keys()].sort(), [...new Set(ROLES.map(typeOf))].sort());
  for (const role of ROLES) {
    const { tools } = byName.get(typeOf(role));
    for (const { prompt } of byRole[role]) {
      const needs = [];
      if (/\bWrite tool\b/.test(prompt)) needs.push("Write");
      if (/\bRead the file\b/.test(prompt)) needs.push("Read");
      if (/\bBash tool\b|`[^`\n]*\b(omni|git)\b[^`\n]*`/.test(prompt)) needs.push("Bash");
      if (role !== "courier" && !/You are read-only/.test(prompt)) needs.push("Edit");
      for (const tool of needs) assert.ok(tools.includes(tool), `${role} needs ${tool}: ${tools}`);
      if (/You are read-only/.test(prompt)) assert.ok(!tools.includes("Edit"), `${role} is read-only but has Edit`);
      if (/use a browser tool/.test(prompt)) assert.ok(tools.includes("mcp__Claude_Browser"), `${role} is told to use a browser`);
    }
  }
});

// ---- the lane -------------------------------------------------------------------------

const laneOf = (dirs) => join(dirs.root, "app-stories", "_lane");
// The run's jobs in the order they started: name, key, command and folder.
const jobsOf = (dirs, run) => {
  const dir = join(dirs.repo, ".omni", "runs", run, "jobs");
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((n) => n.endsWith(".job.json")).map((n) => JSON.parse(readFileSync(join(dir, n), "utf8")))
    .sort((a, b) => Date.parse(a.started) - Date.parse(b.started)).map((j) => ({ name: j.name, key: j.key, cmd: j.cmd, cwd: j.cwd }));
};
// What the script sent a courier, as the payload object.
const payloadOf = (call) => JSON.parse(payloadIn(call.prompt));
// The payload with the parts of the agents' result files it names put in, as
// the helper puts them in.
const filledOf = (call) => {
  const { outs = [], ...payload } = payloadOf(call);
  const results = outs.map((o) => JSON.parse(readFileSync(o.file, "utf8")));
  const fill = (v) => {
    if (Array.isArray(v)) return v.map(fill);
    if (!v || typeof v !== "object") return v;
    if ("$out" in v) return v.at.reduce((x, k) => (x == null ? undefined : x[k]), results[v.$out]);
    if ("$join" in v) return v.$join.map((p) => (typeof p === "string" ? p : String(fill(p) ?? "").trim())).join("");
    return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, fill(x)]));
  };
  return fill(payload);
};
// A commit on deliver/board that this run did not make, as another landing would.
function advance(dirs, file, text) {
  const wt = join(dirs.root, `advance-${++edits}`);
  git(dirs.repo, "worktree", "add", "-q", wt, "deliver/board");
  writeFileSync(join(wt, file), text);
  git(wt, "add", file);
  git(wt, "commit", "-q", "-m", `feat: ${file} on deliver`);
  git(dirs.repo, "worktree", "remove", wt);
}
// A story waiting in reviewed with a worktree on story/<key> and one commit,
// cut before the main checkout's last commit, so that its rebase has work.
function reviewedStory(dirs, key) {
  git(dirs.repo, "worktree", "add", "-q", "-b", `story/${key}`, dirs.worktree(key));
  commit(dirs.worktree(key), `${key}.txt`);
}

test("the lane runs a reviewed story's checks as jobs in the lane worktree, fast, stack_up and stack in turn; a verifier on opus runs the story there, with the seed command as this story and run, a test author on opus writes its tests there, lane-clean throws away what the author left uncommitted before the author's check runs as a job, and lane-land moves deliver/<slug> to the tested commit: the story is done, its worktree gone, and the main checkout untouched", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const lane = laneOf(dirs);
  const check = "test ! -f stray.txt && git diff --quiet";
  const roles = {
    "tests:US-1": [async (call, serial) => {
      await serial(() => {
        commit(lane, "badge.test");
        // Left behind: an untracked file and an edit, which the check must not see.
        writeFileSync(join(lane, "stray.txt"), "stray\n");
        writeFileSync(join(lane, "US-1.txt"), "edited\n");
      });
      return authored({ check, added: [{ test: "badge.test", file: "badge.test", risk: "", overlapCheck: "", sensitivity: "" }] });
    }],
  };
  const head = git(dirs.repo, "rev-parse", "HEAD").trim();
  const checks = { fast: "true", stack_up: "true", stack: "true", seed: "./seed --story", lint: "./lint" };

  const { result, error, calls, logs } = await deliver(dirs, { checks }, roles);

  assert.equal(error, null);
  assert.deepEqual(jobsOf(dirs, result.run), ["fast", "stack_up", "stack"].map((name) => ({ name, key: "US-1", cmd: "true", cwd: lane })).concat([{ name: "tests", key: "US-1", cmd: check, cwd: lane }]));
  const couriers = couriersOf(calls).map((c) => c.label);
  assert.ok(couriers.indexOf("courier:lane-clean:US-1") < couriers.lastIndexOf("courier:job-start:US-1"), couriers.join("\n"));
  assert.ok(logs.includes("US-1: lane-clean threw away 2 files the test author left uncommitted: US-1.txt, stray.txt"), logs.join("\n"));
  const tip = git(dirs.repo, "rev-parse", "refs/heads/deliver/board").trim();
  assert.deepEqual(git(dirs.repo, "log", "--format=%s", `${head}..${tip}`).trim().split("\n"), ["feat: badge.test", "feat: US-1.txt"]);
  assert.deepEqual([dirs.story("US-1").status, existsSync(dirs.worktree("US-1"))], ["done", false]);
  assert.deepEqual([git(dirs.repo, "rev-parse", "HEAD").trim(), git(dirs.repo, "rev-parse", "--abbrev-ref", "HEAD").trim(), git(dirs.repo, "status", "--porcelain")], [head, "main", ""]);
  assert.deepEqual([result.landed, result.tip, result.outcomes], [["US-1"], tip, [{ key: "US-1", outcome: "done" }]]);
  const [verify] = agentsOf(calls, "verify:US-1");
  const [tests] = agentsOf(calls, "tests:US-1");
  assert.deepEqual([verify.model, tests.model, verify.phase, tests.phase], ["opus", "opus", "Lane", "Lane"]);
  for (const prompt of [verify.prompt, tests.prompt]) {
    assert.ok(prompt.includes(`The lane worktree ${lane} holds story/US-1 rebased onto deliver/board, on a detached HEAD`), prompt);
    assert.ok(prompt.includes("Stay on the detached HEAD: never check out or create a branch."), prompt);
  }
  assert.ok(verify.prompt.includes(`\n- The stack has no users until the seed runs. For a credential, run the seed command as \`OMNI_STORY=US-1 OMNI_RUN=${result.run} sh -c './seed --story'\` in ${lane}: it prints what to sign in with. Never write into the stack's database yourself.\n- A criterion about a command is checked by running that command: fast is \`true\`, stack is \`true\`, lint is \`./lint\`.\n`), verify.prompt);
  assert.match(verify.prompt, /You are read-only/);
  assert.ok(tests.prompt.includes(`\`git diff ${head}..HEAD\``), tests.prompt);
});

test("a test author that checked deliver/<slug> out in the lane and committed there costs the story nothing: lane-land refuses the lane it left and puts the branch back at the base, and the verified story lands without the author's commit at its first attempt, which the report notes", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const lane = laneOf(dirs);
  let stray = null;
  const roles = {
    "tests:US-1": [async (call, serial) => {
      await serial(() => {
        git(lane, "checkout", "-q", "deliver/board");
        commit(lane, "stray.test");
        stray = git(lane, "rev-parse", "HEAD").trim();
      });
      return authored();
    }],
  };
  const base = git(dirs.repo, "rev-parse", "HEAD").trim();

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const tip = git(dirs.repo, "rev-parse", "refs/heads/deliver/board").trim();
  assert.throws(() => git(dirs.repo, "merge-base", "--is-ancestor", stray, tip));
  assert.deepEqual([git(dirs.repo, "log", "--format=%s", `${base}..${tip}`).trim(), findingsOf(dirs, "US-1"), agentsOf(calls, "impl:US-1").length], ["feat: US-1.txt", [], 1]);
  const note = `US-1 landed without regression tests: lane-land refused the lane the test author left: the lane worktree had deliver/board checked out, not a detached HEAD: a commit there moves that branch without the compare-and-swap. deliver/board is back at the base ${base.slice(0, 7)}, from ${stray.slice(0, 7)}, and the lane is now detached.`;
  assert.deepEqual(result.notes, [note]);
  assert.ok(readFileSync(result.report, "utf8").includes(`\n${note}\n`), readFileSync(result.report, "utf8"));
  assert.deepEqual([dirs.story("US-1").status, result.landed, result.tip, result.outcomes], ["done", ["US-1"], tip, [{ key: "US-1", outcome: "done" }]]);
});

test("the verifier's prompt leaves out the seed and lint commands when the checks set none, and says all the same that the stack has no users until the seed runs and never to write into its database; a test obligation is the test author's: the verifier skips it, and the story lands without it judged", { timeout: 120000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the badge", human_check: false }];
  const dirs = scratch(t, [story("US-1", { criteria })], [], { checks: { fast: "true" } });
  const roles = { "clarity:US-1": [{ testObligations: [1], gaps: [] }], "verify:US-1": [verified({ 1: "No test ran" })] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const [verify] = agentsOf(calls, "verify:US-1");
  assert.ok(verify.prompt.includes("\n- The stack has no users until the seed runs, and this run has no seed command. Never write into the stack's database yourself.\n"), verify.prompt);
  assert.doesNotMatch(verify.prompt, /seed command as|OMNI_STORY|lint|the stack is up/);
  assert.ok(verify.prompt.includes("\n- A criterion about a command is checked by running that command: fast is `true`.\n"), verify.prompt);
  assert.equal(owedIn(verify.prompt), "Criterion 1 is a test obligation.");
  const [author] = agentsOf(calls, "tests:US-1");
  assert.equal(owedIn(author.prompt), "Criterion 1 is a test obligation.");
  assert.ok(author.prompt.includes("\n## Criteria the verifier confirmed (observed values are withheld so they cannot shape your oracle)\nCriterion 0 of the story.\n"), author.prompt);
  assert.deepEqual([result.outcomes, findingsOf(dirs, "US-1")], [[{ key: "US-1", outcome: "done" }], []]);
});

test("no prompt needs a skill this machine may lack: the verifier opens a page with a browser tool if one is available, and the test author's policy carries the rules of writing tests itself, naming the writing-tests skill as optional", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);

  const { error, calls } = await deliver(dirs);

  assert.equal(error, null);
  const [impl] = agentsOf(calls, "impl:US-1");
  const [verify] = agentsOf(calls, "verify:US-1");
  const [tests] = agentsOf(calls, "tests:US-1");
  assert.ok(verify.prompt.includes(": call the endpoint, run the command, and for a criterion about a page, open the page: use a browser tool if one is available. Reading code is not a check."), verify.prompt);
  assert.ok(tests.prompt.includes("\nThese rules are complete. The writing-tests skill (~/.claude/skills/writing-tests/SKILL.md), where this\nmachine has it, explains them at more length; reading it is optional.\n"), tests.prompt);
  for (const rule of ["THE ORACLE TABLE: ", "Rank the rows, highest first: ", "THE INVENTORY: ", "A TEST: ", "PROVE IT GOES RED: ", "NO TARGET COUNT: "]) assert.ok(tests.prompt.includes(rule), rule);
  assert.doesNotMatch(tests.prompt, /skill step|skill's search|FIRST read/);
  for (const prompt of [impl.prompt, verify.prompt]) assert.doesNotMatch(prompt, /dev-browser|writing-tests/);
});

test("a rebase conflict in lane-prepare sends the story back with no attempt used and the conflict as a finding; its next implementer, still on attempt 1, starts again from deliver/<slug>, and the story lands", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        writeFileSync(join(wt, "README"), "the story's README\n");
        git(wt, "commit", "-q", "-am", "feat: README");
        advance(dirs, "README", "deliver's README\n");
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }, async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        git(wt, "reset", "-q", "--hard", "refs/heads/deliver/board");
        commit(wt, "src/badge.rs");
      });
      return { status: "ready", summary: "Rebuilt.", decisions: [], commits: [] };
    }],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", "rebase conflict in README: story/US-1 no longer rebases onto deliver/board; rebase it onto deliver/board in its worktree and resolve the conflicts", ["README"], true]]);
  assert.deepEqual(timeline(dirs, "US-1"), [
    "claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move backlog",
    "claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move done",
  ]);
  const impls = agentsOf(calls, "impl:US-1");
  assert.deepEqual(impls.map((c) => [c.model, lineAfter(c.prompt, "## Your task now\n").startsWith("This is attempt 1 of 3 since ")]), [["sonnet", true], ["sonnet", true]]);
  assert.ok(impls[1].prompt.includes("The story has 0 open findings of earlier attempts, 1 open finding that used no attempt, and 0 findings from before the human's latest answer. Fix every open one."), impls[1].prompt);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("a rebase conflict in more than 20 files names the first 20 in its finding, which lane-prepare gives, and counts the rest", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const names = Array.from({ length: 21 }, (_, i) => `f${String(i).padStart(2, "0")}.rs`);
  const write = (cwd, text) => {
    for (const name of names) writeFileSync(join(cwd, name), text);
    git(cwd, "add", ...names);
    git(cwd, "commit", "-q", "-m", `feat: ${text.trim()}`);
  };
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        write(worktreeIn(call.prompt), "the story's\n");
        const other = join(dirs.root, "other");
        git(dirs.repo, "worktree", "add", "-q", other, "deliver/board");
        write(other, "deliver's\n");
        git(dirs.repo, "worktree", "remove", other);
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }, async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        git(wt, "reset", "-q", "--hard", "refs/heads/deliver/board");
        commit(wt, "src/badge.rs");
      });
      return { status: "ready", summary: "Rebuilt.", decisions: [], commits: [] };
    }],
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", `rebase conflict in ${names.slice(0, 20).join(", ")} and 1 more: story/US-1 no longer rebases onto deliver/board; rebase it onto deliver/board in its worktree and resolve the conflicts`, names.slice(0, 20), true]]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "done" }]);
});

test("lane-prepare's dirty and wrong_branch use an attempt, with the reason as the finding, and missing uses none: a story with uncommitted changes is fixed on attempt 2, one whose worktree is gone starts again from deliver/<slug>, and one on another branch is back on story/<key> for its attempt 2", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { status: "reviewed" }), story("US-3")]);
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        commit(wt, "US-1.txt");
        writeFileSync(join(wt, "README"), "half done\n");
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }, async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        git(wt, "checkout", "-q", "--", "README");
        commit(wt, "US-1.txt");
      });
      return { status: "ready", summary: "Cleaned up.", decisions: [], commits: [] };
    }],
    "impl:US-3": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        git(wt, "checkout", "-q", "-b", "side");
        commit(wt, "US-3.txt");
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const outcome = (key) => result.outcomes.find((o) => o.key === key);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", `[attempt 1] ${dirs.worktree("US-1")} has uncommitted changes in 1 file, which would not land: commit or discard them`, ["README"], true]]);
  assert.deepEqual([agentsOf(calls, "impl:US-1").map((c) => c.model), outcome("US-1")], [["sonnet", "opus"], { key: "US-1", outcome: "done" }]);
  assert.deepEqual(findingsOf(dirs, "US-2"), [["testing", `no worktree ${dirs.worktree("US-2")}: nothing to land on deliver/board; start again from deliver/board`, [], true]]);
  assert.deepEqual([timeline(dirs, "US-2").slice(0, 3), agentsOf(calls, "impl:US-2").map((c) => lineAfter(c.prompt, "## Your task now\n").slice(0, 22)), outcome("US-2")], [["claim testing", "move backlog", "claim implementing"], ["This is attempt 1 of 3"], { key: "US-2", outcome: "done" }]);
  const [finding] = findingsOf(dirs, "US-3");
  assert.match(finding[1], new RegExp(`^\\[attempt 1\\] ${dirs.worktree("US-3")} has the branch side checked out, not story/US-3: only story/US-3 lands, so its 1 commit not on story/US-3 would be lost \\([0-9a-f]{7} feat: US-3\\.txt\\)\\. Commit on story/US-3$`));
  assert.deepEqual([agentsOf(calls, "impl:US-3").length, outcome("US-3"), questions(dirs)], [2, { key: "US-3", outcome: "done" }, []]);
});

test("trouble in lane-prepare that is not the story's stops the lane for the rest of the run with one thread for the run: the story goes back to reviewed with no attempt used, and the next reviewed story waits there unclaimed", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" }), story("US-2", { status: "reviewed" })]);
  reviewedStory(dirs, "US-1");
  commit(dirs.repo, "other.rs");
  // A hook that refuses every rebase.
  writeFileSync(join(dirs.repo, ".git", "hooks", "pre-rebase"), "#!/bin/sh\necho no rebase today >&2\nexit 1\n");
  chmodSync(join(dirs.repo, ".git", "hooks", "pre-rebase"), 0o755);

  const { result, error } = await deliver(dirs);

  assert.equal(error, null);
  const [stop] = questions(dirs);
  assert.deepEqual([questions(dirs).length, stop.title], [1, `Run ${result.run}: the lane stopped`]);
  assert.match(stop.posts[0].body, new RegExp(`^The lane stopped for the rest of run ${result.run}, on US-1: lane-prepare found trouble that is not the story's: the rebase of story/US-1 onto deliver/board failed: [^\\n]*no rebase today`));
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").holder, dirs.story("US-1").findings || []], ["reviewed", null, []]);
  assert.deepEqual([claimsOf(dirs, "US-1"), claimsOf(dirs, "US-2"), dirs.story("US-2").status], [["testing"], [], "reviewed"]);
  assert.match(result.lane, /^lane-prepare found trouble that is not the story's: /);
  assert.ok(result.notes.some((n) => n.startsWith(`The lane stopped on US-1: lane-prepare found trouble that is not the story's: `) && n.endsWith(` (thread ${stop.id}).`)), result.notes.join("\n"));
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]).sort(), [["US-1", "lane_stopped"], ["US-2", "reviewed"]]);
});

test("a check that fails on the story runs once more on the base, deliver/<slug> without it: passing there, the story uses an attempt with a finding that quotes the end of the check's log, and the lane goes on; the stack is not rebuilt, since the story's build never reached it", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const fast = "if [ -f US-1.txt ]; then echo US-1.txt breaks the build; exit 1; fi";
  const roles = {
    "impl:US-1": [built("US-1.txt"), async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        git(wt, "rm", "-q", "US-1.txt");
        commit(wt, "src/badge.rs");
      });
      return { status: "ready", summary: "Fixed.", decisions: [], commits: [] };
    }],
  };

  const { result, error, calls } = await deliver(dirs, { checks: { fast, stack_up: "true", stack: "true" } }, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", `[attempt 1] The fast check (\`${fast}\`) failed. It passes on deliver/board without the story.\n\nThe end of its output:\n\`\`\`\nUS-1.txt breaks the build\n\`\`\``, [], true]]);
  assert.deepEqual(jobsOf(dirs, result.run).map((j) => j.name), ["fast", "fast", "fast", "stack_up", "stack"]);
  assert.equal(agentsOf(calls, "courier:lane-base:US-1").length, 1);
  assert.deepEqual([dirs.story("US-1").status, result.lane, questions(dirs)], ["done", null, []]);
});

test("a check that fails on the story and again on the base stops the lane for the rest of the run, with one thread for the run that quotes the end of the base's log: the story goes back to reviewed with no attempt used, and a story reviewed after it waits there", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const stack = "echo the stack lost its database; exit 1";
  const lane = laneOf(dirs);

  const { result, error } = await deliver(dirs, { checks: { fast: "true", stack_up: "true", stack } });

  assert.equal(error, null);
  const [stop] = questions(dirs);
  assert.deepEqual([questions(dirs).length, stop.title], [1, `Run ${result.run}: the lane stopped`]);
  const [first] = result.outcomes.filter((o) => o.outcome === "lane_stopped").map((o) => o.key);
  const second = first === "US-1" ? "US-2" : "US-1";
  assert.match(stop.posts[0].body, new RegExp(`^The lane stopped for the rest of run ${result.run}, on ${first}: the stack check failed on it, and the stack check on deliver/board without it\\.\\n\\nOn deliver/board at [0-9a-f]{7}, \`echo the stack lost its database; exit 1\` failed\\.\\n\\nIf a story left state on the stack, [^\\n]+\\n\\nThis is not the story's trouble: [^\\n]+\\n\\nThe end of its output:\\n\`\`\`\\nthe stack lost its database\\n\`\`\`\\n$`));
  // On the base, the stack check ran after a stack_up there.
  assert.deepEqual(jobsOf(dirs, result.run).map((j) => [j.name, j.key, j.cwd]), ["fast", "stack_up", "stack", "stack_up", "stack"].map((name) => [name, first, lane]));
  assert.deepEqual([first, second].map((key) => [dirs.story(key).status, dirs.story(key).findings || []]), [["reviewed", []], ["reviewed", []]]);
  assert.deepEqual([claimsOf(dirs, second).includes("testing"), result.outcomes.find((o) => o.key === second).outcome], [false, "reviewed"]);
  assert.deepEqual(result.lane, "the stack check failed on it, and the stack check on deliver/board without it");
});

test("after a lane failure the lane worktree goes back to the tip, and the stack, which holds the story's build after its stack_up, is rebuilt there as a job; the verifier's unmet criterion uses an attempt", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const stackUp = "git rev-parse HEAD >> ../stack.log";
  const roles = { "verify:US-1": [verified({ 0: "It is pink" })] };
  const start = git(dirs.repo, "rev-parse", "HEAD").trim();

  const { result, error } = await deliver(dirs, { checks: { fast: "true", stack_up: stackUp, stack: "true" } }, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", '[attempt 1] Criterion not met when run: "The badge is red". It is pink', [], true]]);
  assert.deepEqual(jobsOf(dirs, result.run).map((j) => j.name), ["fast", "stack_up", "stack", "rebuild", "fast", "stack_up", "stack"]);
  const built = readFileSync(join(dirs.root, "app-stories", "stack.log"), "utf8").trim().split("\n");
  assert.equal(built.length, 3);
  assert.deepEqual([git(dirs.repo, "log", "-1", "--format=%s", built[0]).trim(), built[1], built[2]], ["feat: US-1.txt", start, git(dirs.repo, "rev-parse", "refs/heads/deliver/board").trim()]);
  assert.equal(dirs.story("US-1").status, "done");
});

test("a job runs to its end however long it takes: job-wait is sent again while the job runs, each wait within a courier's Bash call; the real wait is 480 s", { timeout: 120000 }, async (t) => {
  const source = capped({ JOB_WAIT: 1 });
  const dirs = scratch(t, [story("US-1")]);

  const { result, error, calls } = await deliver(dirs, { checks: { fast: "sleep 3" } }, {}, { source });

  assert.equal(error, null);
  const [fast] = jobsOf(dirs, result.run);
  assert.equal(fast.name, "fast");
  const waits = agentsOf(calls, "courier:job-wait:US-1").map(payloadOf).filter((p) => p.id.endsWith("-fast-1"));
  assert.ok(waits.length >= 2, `${waits.length} waits`);
  assert.ok(waits.every((p) => p.maxSeconds === 1), JSON.stringify(waits));
  assert.equal(dirs.story("US-1").status, "done");
});

test("a lane agent that returns nothing is asked again after 30 s and after 2 minutes: a verifier that returns nothing 3 times sends its story back to reviewed with no attempt used, and the lane goes back to the tip", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const roles = { "verify:US-1": [null, null, null] };

  const { result, error, calls, sleeps } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual([agentsOf(calls, "verify:US-1").length, sleeps], [3, [30, 120]]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "no_result", detail: "the verifier returned nothing, 3 times; the story is released without using an attempt" }]);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").holder, dirs.story("US-1").findings || []], ["reviewed", null, []]);
  assert.equal(agentsOf(calls, "courier:lane-reset:US-1").length, 1);
  const lane = laneOf(dirs);
  assert.deepEqual([git(lane, "rev-parse", "HEAD").trim(), git(lane, "status", "--porcelain")], [git(dirs.repo, "rev-parse", "refs/heads/deliver/board").trim(), ""]);
});

test("a test author's question goes to the council: settled, the author runs again with the decision and the story lands; not settled, it opens a thread that blocks the story in reviewed, and the lane goes back to the tip", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const asks = (question) => authored({ status: "needs_decision", question, options: ["Yes", "No"], recommendation: "No" });
  const quoted = vote(1, { quote: "Badges blink", source: "doc doc-spec node n1" });
  const roles = {
    "tests:US-1": [asks("Does the badge blink?"), authored()],
    "tests:US-2": [asks("Does the count wrap?")],
    ...councilOf("US-1", [quoted], [vote(1)], [vote(1)]),
    ...councilOf("US-2", [vote(1)], [vote(2)], [vote(0)]),
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const authors = agentsOf(calls, "tests:US-1");
  assert.equal(authors.length, 2);
  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Does the badge blink?", choice: "Yes (council: \"Badges blink\" (doc doc-spec node n1))", by: "agent" }]);
  assert.match(lineAfter(authors[1].prompt, "## Your task now\n"), /^An earlier session stopped to ask a question, which the story's decisions now settle\./);
  for (const c of agentsOf(calls, "council:US-1:code")) assert.ok(c.prompt.includes(`You read the code, and nothing else: the story's worktree ${laneOf(dirs)}.`), c.prompt);
  const [asked] = questions(dirs);
  assert.deepEqual([questions(dirs).length, asked.title], [1, "US-2: Does the count wrap?"]);
  assert.match(asked.posts[0].body, /\nAsked by the test author\.\n/);
  const outcome = (key) => result.outcomes.find((o) => o.key === key);
  assert.deepEqual([outcome("US-1"), outcome("US-2")], [{ key: "US-1", outcome: "done" }, { key: "US-2", outcome: "needs_decision", thread: asked.id }]);
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").blocked], ["reviewed", true]);
  assert.deepEqual(result.council, { settled: 1, majority: 0, asked: 1 });
});

test("agents that return nothing for 3 stories in a row stop the run taking stories, with one thread for the run; each story goes back with no attempt used", { timeout: 120000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  const roles = Object.fromEntries(keys.map((key) => [`impl:${key}`, [null, null, null]]));

  const { result, error, logs } = await deliver(dirs, { implementers: 1 }, roles);

  assert.equal(error, null);
  const [row] = questions(dirs);
  assert.deepEqual([questions(dirs).length, row.title], [1, `Run ${result.run}: agents return nothing`]);
  assert.match(row.posts[0].body, /^Agents of run \S+ returned nothing for US-1, US-2, US-3, in a row, each after 2 retries over 150 s; the stories went back to their waiting states with no attempt used\.\n/);
  assert.deepEqual([result.halted, claimsOf(dirs, "US-4")], ["agents returned nothing for US-1, US-2, US-3, in a row", []]);
  assert.ok(logs.includes("The run takes no new story: agents returned nothing for US-1, US-2, US-3, in a row"), logs.join("\n"));
  assert.ok(result.notes.includes(`The run took no new story once agents returned nothing for US-1, US-2, US-3, in a row (thread ${row.id}).`), result.notes.join("\n"));
  for (const key of keys.slice(0, 3)) assert.deepEqual([dirs.story(key).status, dirs.story(key).findings || [], dirs.story(key).blocked], ["backlog", [], false], key);
});

test("an agent that returns something ends the row: two stories sent back for nothing, then one whose implementer answers, then one more sent back, do not stop the run", { timeout: 120000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4", "US-5"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  const roles = Object.fromEntries(["US-1", "US-2", "US-4"].map((key) => [`impl:${key}`, [null, null, null]]));

  const { result, error } = await deliver(dirs, { implementers: 1 }, roles);

  assert.equal(error, null);
  assert.deepEqual([result.halted, questions(dirs)], [null, []]);
  assert.deepEqual(claimsOf(dirs, "US-5")[0], "implementing");
});

test("a job is stopped when the lane throws: a story whose job-wait courier fails has its fast check stopped before the next story's turn in the lane starts, the lane goes back to the tip, and the story gets its thread", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const fast = 'm=$(git log -1 --format=%s); echo "start $m" >> ../jobs.log; sleep 6; echo "end $m" >> ../jobs.log';
  let victim = null;
  const fault = (call) => {
    if (!call.label.startsWith("courier:job-wait:")) return null;
    const key = call.label.split(":")[2];
    if (!victim) victim = key;
    return key === victim ? "nothing" : null;
  };

  const { result, error, calls } = await deliver(dirs, { implementers: 1, checks: { fast } }, {}, { fault });

  assert.equal(error, null);
  const other = victim === "US-1" ? "US-2" : "US-1";
  // The stopped job never ends; it can be stopped before its command writes its first line.
  const jobs = readFileSync(join(dirs.root, "app-stories", "jobs.log"), "utf8").trim().split("\n");
  assert.deepEqual(jobs.filter((l) => l !== `start feat: ${victim}.txt`), [`start feat: ${other}.txt`, `end feat: ${other}.txt`]);
  const labels = couriersOf(calls).map((c) => c.label);
  // The lane's turns go one at a time: the next one starts with lane-prepare.
  assert.ok(labels.indexOf(`courier:job-kill:${victim}`) < labels.indexOf(`courier:lane-reset:${victim}`), labels.join("\n"));
  assert.ok(labels.indexOf(`courier:lane-reset:${victim}`) < labels.indexOf(`courier:lane-prepare:${other}`), labels.join("\n"));
  const killed = JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", result.run, "jobs", `${victim}-fast-1.json`), "utf8"));
  assert.equal(killed.ok, false);
  const outcome = (key) => result.outcomes.find((o) => o.key === key);
  assert.deepEqual([outcome(victim).outcome, outcome(victim).detail, outcome(other).outcome], ["failed", `courier:job-wait:${victim}: the courier returned nothing, after 3 retries`, "done"]);
  assert.deepEqual(questions(dirs).map((th) => th.title), [`${victim}: delivery stopped on an error`]);
});

test("a job whose job-start throws is stopped all the same: the helper started it, its courier changed the answer every time, and job-kill finds the job by the start's opId before the lane goes back to the tip", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const fault = (call) => (call.label === "courier:job-start:US-1" ? "output" : null);

  const { result, error, calls } = await deliver(dirs, { checks: { fast: "sleep 30" } }, {}, { fault });

  assert.equal(error, null);
  const labels = couriersOf(calls).map((c) => c.label);
  const kill = labels.indexOf("courier:job-kill:US-1");
  assert.ok(kill !== -1 && kill < labels.indexOf("courier:lane-reset:US-1"), labels.join("\n"));
  assert.deepEqual(payloadOf(couriersOf(calls)[kill]), { opId: "job-kill-US-1-1", start: "job-start-US-1-1" });
  const stopped = JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", result.run, "jobs", "US-1-fast-1.json"), "utf8"));
  assert.deepEqual([stopped.ok, stopped.signal], [false, "SIGTERM"]);
  // Stopped by that job-kill, not by finish at the end of the run.
  const reset = readFileSync(join(dirs.repo, ".omni", "runs", result.run, "timings.jsonl"), "utf8").trim().split("\n").map((l) => JSON.parse(l)).find((r) => r.op === "lane-reset-US-1-1");
  assert.ok(Date.parse(stopped.ended) <= Date.parse(reset.started), `${stopped.ended} ${reset.started}`);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome, o.detail]), [["US-1", "failed", "courier:job-start:US-1: its output does not match its hash: the courier changed it, after 3 retries"]]);
});

test("a story that reaches the hard cap in the lane stops there: the job it waits on is stopped, the lane goes back to the tip, and the story is released with no attempt used", { timeout: 120000 }, async (t) => {
  // About 25 calls come before the lane's first job; each wait after that is one more.
  const source = capped({ AGENT_CAP: 25, HARD_CAP: 32, STORY_CALLS: 1, CHECK_CALLS: 1, JOB_WAIT: 1 });
  const dirs = scratch(t, [story("US-1")]);

  const { result, error, calls } = await deliver(dirs, { checks: { fast: "sleep 30" } }, {}, { source });

  assert.equal(error, null);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "stopped", detail: "stopped at the agent cap of 32" }]);
  const labels = couriersOf(calls).map((c) => c.label);
  const kill = labels.indexOf("courier:job-kill:US-1");
  assert.ok(kill !== -1 && kill < labels.indexOf("courier:lane-reset:US-1") && labels.indexOf("courier:lane-reset:US-1") < labels.indexOf("courier:release:US-1"), labels.join("\n"));
  const [fast] = readdirSync(join(dirs.repo, ".omni", "runs", result.run, "jobs")).filter((n) => /^US-1-fast-\d+\.json$/.test(n));
  assert.equal(JSON.parse(readFileSync(join(dirs.repo, ".omni", "runs", result.run, "jobs", fast), "utf8")).ok, false);
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.findings || []], ["reviewed", null, []]);
  const lane = laneOf(dirs);
  assert.deepEqual([git(lane, "rev-parse", "HEAD").trim(), git(lane, "status", "--porcelain")], [git(dirs.repo, "rev-parse", "refs/heads/deliver/board").trim(), ""]);
  assert.ok(result.agents <= 32 + 8, `${result.agents} calls`);
});

test("a rebase the implementer left in progress in the story's worktree is the story's: lane-prepare gives dirty, the story uses an attempt and the lane goes on, and the next implementer gets the worktree as it is, told to finish or abort the rebase", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  let stuck = null;
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        writeFileSync(join(wt, "README"), "the story's README\n");
        git(wt, "commit", "-q", "-am", "feat: README");
        advance(dirs, "README", "deliver's README\n");
        assert.throws(() => git(wt, "rebase", "-q", "deliver/board"));
        stuck = git(wt, "rev-parse", "HEAD").trim();
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }, async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        git(wt, "rebase", "--abort");
        git(wt, "reset", "-q", "--hard", "refs/heads/deliver/board");
        commit(wt, "src/badge.rs");
      });
      return { status: "ready", summary: "Rebuilt.", decisions: [], commits: [] };
    }],
  };

  const { result, error, calls } = await deliver(dirs, { implementers: 1 }, roles);

  assert.equal(error, null);
  const wt = dirs.worktree("US-1");
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", `[attempt 1] a rebase of story/US-1 is in progress in ${wt}: finish it or abort it, and commit the story's work on story/US-1`, ["README"], true]]);
  const impls = agentsOf(calls, "impl:US-1");
  assert.ok(impls[1].prompt.includes(`\n${wt} has a detached HEAD at ${stuck.slice(0, 7)} checked out, not story/US-1, which is what lands, with a rebase of story/US-1 in progress, so the workflow left it as it is: finish the rebase or abort it, check story/US-1 out, and commit the story's work there.\n`), impls[1].prompt);
  assert.deepEqual([result.lane, questions(dirs), result.outcomes.map((o) => [o.key, o.outcome]).sort()], [null, [], [["US-1", "done"], ["US-2", "done"]]]);
});

test("a story's rebase conflicts send it back with no attempt used twice in a run; from the third, one its implementer resolves by a merge, which the lane's rebase meets again, uses an attempt, so the story ends on its thread instead of cycling to the agent cap", { timeout: 300000 }, async (t) => {
  // Low caps, so that a story that cycles stops soon.
  const source = capped({ AGENT_CAP: 150, HARD_CAP: 200 });
  const dirs = scratch(t, [story("US-1")]);
  const first = async (call, serial) => {
    await serial(() => {
      const wt = worktreeIn(call.prompt);
      writeFileSync(join(wt, "README"), "the story's README\n");
      git(wt, "commit", "-q", "-am", "feat: README");
      advance(dirs, "README", "deliver's README\n");
    });
    return { status: "ready", summary: "Built.", decisions: [], commits: [] };
  };
  const merge = async (call, serial) => {
    await serial(() => {
      const wt = worktreeIn(call.prompt);
      try {
        git(wt, "merge", "-q", "--no-edit", "deliver/board");
      } catch {
        writeFileSync(join(wt, "README"), "the story's README\n");
        git(wt, "add", "README");
        git(wt, "commit", "-q", "--no-edit");
      }
    });
    return { status: "ready", summary: "Merged.", decisions: [], commits: [] };
  };

  const { result, error, calls } = await deliver(dirs, {}, { "impl:US-1": [first, ...Array(8).fill(merge)] }, { source });

  assert.equal(error, null);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome]), [["US-1", "attempts_exhausted"]]);
  assert.deepEqual(agentsOf(calls, "impl:US-1").map((c) => c.model), ["sonnet", "sonnet", "sonnet", "opus", "opus"]);
  const conflict = "rebase conflict in README: story/US-1 no longer rebases onto deliver/board; rebase it onto deliver/board in its worktree and resolve the conflicts";
  assert.deepEqual(findingsOf(dirs, "US-1").map(([, text]) => text), [conflict, conflict, `[attempt 1] ${conflict}`, `[attempt 2] ${conflict}`, `[attempt 3] ${conflict}`]);
  assert.deepEqual(questions(dirs).map((th) => th.title), ["US-1: still failing after 3 attempts"]);
});

// A stack whose database keeps the migrations it applied, as omnistate's test
// stack does, and whose API refuses one the tree lacks, as sqlx's migrate does.
const STACK_UP = 'for f in $(cat ../db 2>/dev/null); do [ -f "$f" ] || { echo "migration $f was previously applied but is missing"; exit 1; }; done; ls migrations/*.sql 2>/dev/null >> ../db; sort -u ../db -o ../db 2>/dev/null; true';

test("a story whose stack check fails leaves its migration in the stack: with stack_reset, the rerun on the base resets the stack first, so the failure is the story's, which uses an attempt, and the lane goes on", { timeout: 120000 }, async (t) => {
  // US-2 waits for US-1, whose stack_up is the run's first, after a stack_reset.
  const dirs = scratch(t, [story("US-1"), story("US-2", { blocked_by: ["US-1"] })]);
  const stack = 'if [ -f migrations/US-1.sql ]; then echo "US-1 breaks the stack tests"; exit 1; fi';
  const roles = { "impl:US-1": [built("migrations/US-1.sql"), async (call, serial) => {
    await serial(() => {
      const wt = worktreeIn(call.prompt);
      git(wt, "rm", "-q", "migrations/US-1.sql");
      commit(wt, "src/US-1.rs");
    });
    return { status: "ready", summary: "Fixed.", decisions: [], commits: [] };
  }] };

  const { result, error } = await deliver(dirs, { implementers: 1, checks: { fast: "true", stack_up: STACK_UP, stack, stack_reset: "rm -f ../db" } }, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", `[attempt 1] The stack check (\`${stack}\`) failed. It passes on deliver/board without the story.\n\nThe end of its output:\n\`\`\`\nUS-1 breaks the stack tests\n\`\`\``, [], true]]);
  assert.deepEqual(jobsOf(dirs, result.run).filter((j) => j.key === "US-1").map((j) => j.name), ["fast", "stack_reset", "stack_up", "stack", "stack_reset", "stack_up", "stack", "fast", "stack_up", "stack"]);
  assert.deepEqual([result.lane, questions(dirs), result.outcomes.map((o) => [o.key, o.outcome]).sort()], [null, [], [["US-1", "done"], ["US-2", "done"]]]);
});

test("without stack_reset, a story whose migration stays in the stack stops the lane on the base's stack_up, and the lane-stop thread names that likely cause", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  const stack = 'if [ -f migrations/US-1.sql ]; then echo "US-1 breaks the stack tests"; exit 1; fi';

  const { result, error } = await deliver(dirs, { implementers: 1, checks: { fast: "true", stack_up: STACK_UP, stack } }, { "impl:US-1": [built("migrations/US-1.sql")] });

  assert.equal(error, null);
  assert.equal(result.lane, "the stack check failed on it, and the stack_up check on deliver/board without it");
  const [stop] = questions(dirs);
  assert.ok(stop.posts[0].body.includes(" failed.\n\nIf a story left state on the stack, make stack_reset (or stack_up) reset it: a migration of a story the lane sent back, for one, stays in the stack's database, and fails every stack_up of a tree without it.\n\nThis is not the story's trouble: "), stop.posts[0].body);
  assert.ok(stop.posts[0].body.endsWith("\n\nThe end of its output:\n```\nmigration migrations/US-1.sql was previously applied but is missing\n```\n"), stop.posts[0].body);
});

test("a story the verifier fails leaves its migration in the stack: with stack_reset, the rebuild at the tip resets the stack first, so the next pass of the lane is clean, and both stories land", { timeout: 120000 }, async (t) => {
  // US-2 waits for US-1, whose stack_up is the run's first, after a stack_reset.
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] }), story("US-2", { blocked_by: ["US-1"] })]);
  const roles = { "impl:US-1": [built("migrations/US-1.sql")], "verify:US-1": [verified({ 0: "It is pink" })] };

  const { result, error, logs } = await deliver(dirs, { implementers: 1, checks: { fast: "true", stack_up: STACK_UP, stack: "true", stack_reset: "rm -f ../db" } }, roles);

  assert.equal(error, null);
  assert.deepEqual(jobsOf(dirs, result.run).filter((j) => j.key === "US-1").map((j) => j.name).slice(0, 6), ["fast", "stack_reset", "stack_up", "stack", "stack_reset", "rebuild"]);
  assert.ok(logs.includes("US-1: the stack is rebuilt from deliver/board"), logs.join("\n"));
  assert.deepEqual([result.lane, result.outcomes.map((o) => [o.key, o.outcome]).sort()], [null, [["US-1", "done"], ["US-2", "done"]]]);
});

test("what a stopped run left on the stack costs the next run's first story nothing: with stack_reset set, the run's first stack_up comes after a stack_reset, the only one of the run, and both stories land at their first attempt", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  // A story of the stopped run left this behind, and stack_up fails until stack_reset removes it.
  const leftover = join(dirs.root, "leftover-migration");
  writeFileSync(leftover, "");

  const { result, error } = await deliver(dirs, { implementers: 1, checks: { stack_up: `test ! -e ${leftover}`, stack_reset: `rm -f ${leftover}` } });

  assert.equal(error, null);
  const names = jobsOf(dirs, result.run).map((j) => j.name);
  assert.deepEqual([names.filter((n) => n === "stack_reset").length, names.indexOf("stack_reset") + 1], [1, names.indexOf("stack_up")], names.join(", "));
  assert.deepEqual(["US-1", "US-2"].map((key) => [findingsOf(dirs, key), dirs.story(key).status]), [[[], "done"], [[], "done"]]);
  assert.deepEqual(boardLog(dirs).filter((e) => e.endsWith(" backlog")), []);
});

test("a test author's check that leaves a file behind costs the story nothing: lane-clean runs again after the check, and the story lands with its tests at its first attempt", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const lane = laneOf(dirs);
  const roles = { "tests:US-1": [async (call, serial) => {
    await serial(() => commit(lane, "badge.test"));
    return authored({ check: "echo report > junit.xml", added: [{ test: "badge", file: "badge.test", risk: "", overlapCheck: "", sensitivity: "" }] });
  }] };

  const { result, error, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.ok(logs.includes("US-1: lane-clean threw away 1 file the test author's check left: junit.xml"), logs.join("\n"));
  assert.deepEqual([git(dirs.repo, "log", "-1", "--format=%s", "refs/heads/deliver/board").trim(), result.outcomes, findingsOf(dirs, "US-1"), result.notes], ["feat: badge.test", [{ key: "US-1", outcome: "done" }], [], []]);
});

test("a test author's check that comes out red gets the author once more, told where the check's output is: tests it fixes land with the story, at its first attempt", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const lane = laneOf(dirs);
  const check = "test -f fixed.test || { echo badge.test fails; exit 1; }";
  const roles = { "tests:US-1": [async (call, serial) => {
    await serial(() => commit(lane, "badge.test"));
    return authored({ check, added: [{ test: "badge", file: "badge.test", risk: "", overlapCheck: "", sensitivity: "" }] });
  }, async (call, serial) => {
    await serial(() => commit(lane, "fixed.test"));
    return authored({ check, added: [{ test: "badge", file: "badge.test", risk: "", overlapCheck: "", sensitivity: "" }] });
  }] };
  const head = git(dirs.repo, "rev-parse", "HEAD").trim();

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const authors = agentsOf(calls, "tests:US-1");
  assert.equal(authors.length, 2);
  const log = join(dirs.repo, ".omni", "runs", result.run, "jobs", "US-1-tests-1.log");
  assert.equal(between(authors[1].prompt, "## Your task now\n", "\nReturn raw structured data only."), `An earlier session committed tests in the lane worktree, and their check \`${check}\` failed: its output is in ${log}: read the end of it.\nThe story itself passed its checks and its verifier. Fix the tests, never the production code, so that the check passes, commit the fix, and return the result with the check.`);
  assert.equal(readFileSync(log, "utf8"), "badge.test fails\n");
  assert.deepEqual(git(dirs.repo, "log", "--format=%s", `${head}..refs/heads/deliver/board`).trim().split("\n"), ["feat: fixed.test", "feat: badge.test", "feat: US-1.txt"]);
  assert.deepEqual([result.outcomes, findingsOf(dirs, "US-1"), result.notes], [[{ key: "US-1", outcome: "done" }], [], []]);
});

test("a test author's check still red after the author is asked once more costs the story nothing: its tests are dropped, the verified story lands without them at its first attempt, and the report notes it", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const check = "echo the badge test fails; exit 1";
  // Asked once more, the author gives no check: it is held to the one it gave before.
  const roles = { "tests:US-1": [testsIn(dirs, "badge.test", check), testsIn(dirs, "badge2.test", "")] };
  const head = git(dirs.repo, "rev-parse", "HEAD").trim();

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.equal(agentsOf(calls, "tests:US-1").length, 2);
  assert.deepEqual(git(dirs.repo, "log", "--format=%s", `${head}..refs/heads/deliver/board`).trim(), "feat: US-1.txt");
  const note = `US-1 landed without regression tests: the test author's check \`${check}\` failed again, after the author was asked once more.`;
  assert.deepEqual([result.outcomes, findingsOf(dirs, "US-1"), result.notes], [[{ key: "US-1", outcome: "done" }], [], [note]]);
  assert.ok(readFileSync(result.report, "utf8").includes(`\n${note}\n`), readFileSync(result.report, "utf8"));
  const lane = laneOf(dirs);
  assert.deepEqual([git(lane, "status", "--porcelain"), dirs.story("US-1").status], ["", "done"]);
});

test("a lane that is dirty when lane-land runs, from something the test author left running, is refused as the lane's state: the story lands without its tests at its first attempt, and the report notes it", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const lane = laneOf(dirs);
  let late = false;
  // A file that appears after the last lane-clean, as a process the author left running would write it.
  const fault = (call) => {
    if (call.label === "courier:lane-land:US-1" && !late) {
      late = true;
      writeFileSync(join(lane, "late.log"), "written after the clean\n");
    }
    return null;
  };
  const head = git(dirs.repo, "rev-parse", "HEAD").trim();

  const { result, error } = await deliver(dirs, {}, { "tests:US-1": [testsIn(dirs, "badge.test")] }, { fault });

  assert.equal(error, null);
  assert.deepEqual(git(dirs.repo, "log", "--format=%s", `${head}..refs/heads/deliver/board`).trim(), "feat: US-1.txt");
  assert.deepEqual([result.outcomes, findingsOf(dirs, "US-1"), result.notes], [[{ key: "US-1", outcome: "done" }], [], ["US-1 landed without regression tests: lane-land refused the lane the test author left: the lane worktree has uncommitted changes in 1 file (late.log): what lands must be what was checked, so lane-clean it and check it again."]]);
});

test("a test author whose test finds a defect sends the story back for its next attempt, with the defect as its finding, and the lane goes back to the tip", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const roles = { "tests:US-1": [authored({ status: "found_defect", defect: { test: "badge_is_red", observed: "pink", expected: "red" } })] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [["testing", "[attempt 1] The test author's test badge_is_red found a defect. Observed: pink. Expected: red", [], true]]);
  const labels = couriersOf(calls).map((c) => c.label);
  assert.ok(labels.indexOf("courier:lane-reset:US-1") < labels.indexOf("courier:rework:US-1"), labels.join("\n"));
  assert.deepEqual([agentsOf(calls, "impl:US-1").length, result.outcomes], [2, [{ key: "US-1", outcome: "done" }]]);
});

test("a test author that is blocked opens a thread with its reason, which blocks the story in reviewed with no attempt used, and the lane goes back to the tip", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const roles = { "tests:US-1": [authored({ status: "blocked", blockers: "No test runner for the badge." })] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const [blocked] = questions(dirs);
  assert.deepEqual([blocked.title, blocked.posts[0].body], ["US-1: the test author is blocked", "No test runner for the badge.\n"]);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").blocked, dirs.story("US-1").findings || [], result.outcomes], ["reviewed", true, [], [{ key: "US-1", outcome: "blocked", thread: blocked.id }]]);
  assert.equal(agentsOf(calls, "courier:lane-reset:US-1").length, 1);
});

test("a lane stop whose release fails stops the lane all the same, with its thread, and the report notes the release", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  reviewedStory(dirs, "US-1");
  commit(dirs.repo, "other.rs");
  writeFileSync(join(dirs.repo, ".git", "hooks", "pre-rebase"), "#!/bin/sh\necho no rebase today >&2\nexit 1\n");
  chmodSync(join(dirs.repo, ".git", "hooks", "pre-rebase"), 0o755);
  const fault = (call) => (call.label === "courier:release:US-1" ? "nothing" : null);

  const { result, error } = await deliver(dirs, {}, {}, { fault });

  assert.equal(error, null);
  assert.deepEqual(questions(dirs).map((th) => th.title), [`Run ${result.run}: the lane stopped`]);
  assert.deepEqual(result.outcomes.map((o) => o.outcome), ["lane_stopped"]);
  assert.ok(result.notes.includes("US-1 could not be released when the lane stopped, and stays in testing until the next run's start releases it: courier:release:US-1: the courier returned nothing, after 3 retries"), result.notes.join("\n"));
  assert.equal(dirs.story("US-1").status, "testing");
});

// ---- the end of the run ---------------------------------------------------------------

test("the run ends with the journeys check in the lane worktree, then finish gets the report: every outcome, the council's counts, the journeys and the notes, and the helper adds the blocked stories with their threads from the board, with no courier for them; the report goes on the Run thread, the lock is given up, and the run returns the report's path and the landed state", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3", { blocked: true, links: [{ thread_id: "th-1", label: "question" }] })], [{ id: "th-1", title: "US-3: Which red?", feature: "board", posts: [], conclusion: null }]);
  const roles = {
    "impl:US-2": [{ status: "blocked", summary: "The badge API is missing." }],
    final: [{ healthy: true, journeys: ["Badge, then count: works"], report: "One journey spans them." }],
  };

  const { result, error, calls, phases } = await deliver(dirs, { storyLimit: 2 }, roles);

  assert.equal(error, null);
  const [blocked] = questions(dirs).filter((th) => th.title === "US-2: the implementer is blocked");
  const [final] = agentsOf(calls, "final");
  assert.deepEqual([final.model, final.phase], ["opus", "Report"]);
  assert.ok(final.prompt.includes(`The lane worktree ${laneOf(dirs)}, on a detached HEAD at the tip of deliver/board, where this run landed its stories.`), final.prompt);
  assert.ok(final.prompt.includes("\n## The stories this run landed\n- US-1\n"), final.prompt);
  const [finish] = agentsOf(calls, "courier:finish");
  const { report } = filledOf(finish);
  assert.deepEqual({ ...report, outcomes: [...report.outcomes].sort((a, b) => a.key.localeCompare(b.key)) }, {
    outcomes: [{ key: "US-1", outcome: "done" }, { key: "US-2", outcome: "blocked" }],
    council: { settled: 0, majority: 0, asked: 0 },
    journeys: ["healthy: One journey spans them.", "Badge, then count: works"],
    notes: ["The run took its storyLimit of 2 stories."],
  });
  const posted = dirs.state().threads.find((th) => th.id === result.thread).posts;
  assert.deepEqual([posted.length, posted[0].summary, result.posted], [1, `Report of ${result.run}`, true]);
  assert.equal(posted[0].body, readFileSync(result.report, "utf8"));
  assert.deepEqual(agentsOf(calls, "courier:context:US-3"), []);
  assert.equal(result.report, join(dirs.repo, ".omni", "runs", result.run, "report.md"));
  assert.match(posted[0].body, /\n## Landed on deliver\/board\n- US-1 Story US-1: [0-9a-f]{7} feat: US-1\.txt\n\nBring them into your branch with `git merge --ff-only deliver\/board`\.\n/);
  assert.ok(posted[0].body.includes(`\n## Blocked\n- US-2 Story US-2: thread ${blocked.id}\n- US-3 Story US-3: thread th-1\n`), posted[0].body);
  assert.ok(posted[0].body.includes("\n## Journeys\n- healthy: One journey spans them.\n- Badge, then count: works\n"), posted[0].body);
  assert.deepEqual([result.landed, result.tip], [["US-1"], git(dirs.repo, "rev-parse", "refs/heads/deliver/board").trim()]);
  assert.equal(existsSync(join(dirs.repo, ".omni", "run.lock")), false);
  assert.deepEqual(phases, ["Start", "Unblock", "Clarity", "Deliver", "Report"]);
});

test("the journeys check runs no check itself, nor the stack command, since its Bash call stops after 10 minutes, and is told the lane's last check instead: here the fast check that failed on deliver/<slug> without the story after the one that landed, and stopped the lane", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { blocked_by: ["US-1"] })]);
  const fast = "if [ -e ../red ]; then echo the fast check is red; exit 1; fi";
  // US-2's implementer turns the fast check red for every tree, once US-1 has landed.
  const roles = { "impl:US-2": [(call, serial) => {
    writeFileSync(join(dirs.root, "app-stories", "red"), "");
    return built("US-2.txt")(call, serial);
  }] };

  const { result, error, calls } = await deliver(dirs, { checks: { fast } }, roles);

  assert.equal(error, null);
  assert.deepEqual([result.landed, result.lane], [["US-1"], "the fast check failed on it, and the fast check on deliver/board without it"]);
  const [final] = agentsOf(calls, "final");
  assert.ok(final.prompt.includes(`\n## The lane's checks\nThe lane ran its checks on every story before it landed, so run none of them yourself, not even the fast command: your Bash call stops after 10 minutes, and a check can take longer. The lane's last check, fast (\`${fast}\`) on deliver/board without US-2, failed: its output is in ${join(dirs.repo, ".omni", "runs", result.run, "jobs", "US-2-fast-2.log")}.\n`), final.prompt);
  assert.doesNotMatch(final.prompt, /Run the fast command/);
  assert.ok(final.prompt.includes(" and exercise up to 3 of them for real, with the real components. Report each with its outcome.\n"), final.prompt);
  assert.equal(final.schema.properties.healthy.description, "The lane's last check passed and every cross-story journey worked");
});

test("a journeys check whose lane-reset fails is not run: the report says why, and the run finishes", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const fault = (call) => (call.label === "courier:lane-reset" ? "nothing" : null);

  const { result, error, calls } = await deliver(dirs, {}, {}, { fault });

  assert.equal(error, null);
  assert.deepEqual(agentsOf(calls, "final"), []);
  const [finish] = agentsOf(calls, "courier:finish");
  assert.deepEqual(payloadOf(finish).report.journeys, ["not run: the lane worktree could not be put back at the tip of deliver/board: courier:lane-reset: the courier returned nothing, after 3 retries"]);
  assert.deepEqual([result.outcomes, result.posted], [[{ key: "US-1", outcome: "done" }], true]);
});

test("finish runs whenever start succeeded: a run that throws posts its report, with the error among its notes, and gives the lock up before the throw goes on", { timeout: 120000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  // The board cannot be listed: the deliver loop throws, and finish lists the blocked stories itself.
  const fault = (call) => (call.label === "courier:list" ? "nothing" : null);

  const { error, calls } = await deliver(dirs, {}, {}, { fault });

  assert.equal(error.message, "courier:list: the courier returned nothing, after 3 retries");
  assert.equal(existsSync(join(dirs.repo, ".omni", "run.lock")), false);
  const [run] = dirs.state().threads.filter((th) => /^Run run-\d{8}-\d{6}-\d+$/.test(th.title));
  assert.equal(run.posts.length, 1);
  assert.ok(run.posts[0].body.includes("\nThe run stopped on an error: courier:list: the courier returned nothing, after 3 retries\n"), run.posts[0].body);
  assert.ok(run.posts[0].body.includes("\n## Blocked\n- none\n"), run.posts[0].body);
  assert.equal(agentsOf(calls, "courier:finish").length, 1);
});
