// omni-deliver.js, the native workflow script, run under a fake of the
// Workflow runtime: agent, parallel, pipeline, phase and log with the
// runtime's semantics (parallel is a barrier, pipeline has none, and a thunk
// or a stage that throws gives null), with Date, Date.now and Math.random
// throwing as they do there. The fake agent carries out couriers for real:
// it writes the payload where the prompt says and runs the command, which runs
// workflows/deliver-step.mjs in a temp git repository with a fake omni first
// on PATH (fixtures/fake-omni.mjs) whose state is a JSON file. It answers the
// role agents from canned results by label.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFile, execFileSync, spawn, spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { fnv1a } from "./deliver-step.mjs";

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
const criteriaIn = (prompt) => {
  const text = between(prompt, "## Acceptance criteria (number: text)\n", "\n\n## Your task now");
  if (text === "(none)") return [];
  return text.split("\n").map((l) => l.match(/^(\d+): (.*?)(?: \[(?:human check|test obligation)\])*$/)).map((m) => ({ index: Number(m[1]), criterion: m[2] }));
};
const questionIn = (prompt) => lineAfter(prompt, "## The question\n");

// An implementer that commits a file in its worktree and is ready.
const built = (file, fields = {}) => async (call, serial) => {
  await serial(() => commit(worktreeIn(call.prompt), file));
  return { status: "ready", summary: `Built ${file}.`, decisions: [], commits: [], ...fields };
};
// A reviewer's verdict on every criterion of its prompt: met, but for those
// unmet names with their evidence and the test obligations it names.
const reviewed = (verdict, { unmet = {}, defects = [], obligations = [] } = {}) => (call) => ({
  verdict,
  criteria: criteriaIn(call.prompt).map((c) => ({ ...c, obligation: obligations.includes(c.index), met: !(c.index in unmet), evidence: unmet[c.index] || "Seen in the diff" })),
  defects,
});
const vote = (pick, fields = {}) => ({ pick, quote: "", source: "", minor: false, ...fields });
// The answers of a story's council members, by reading: spec, code and user.
const councilOf = (key, spec, code, user) => ({ [`council:${key}:spec`]: spec, [`council:${key}:code`]: code, [`council:${key}:user`]: user });
const gap = (question, impact, fields = {}) => ({ criterion: "The badge has a colour", question, options: ["Red", "Blue"], recommendation: "Blue", reason: "\"warning colour\"", blocking: true, impact, ...fields });

// What a role agent answers when its test gives it nothing: a clarity check
// with no gap, an implementer that commits a file, a reviewer that passes
// every criterion, and a council member with no pick.
const byDefault = (call, serial) => {
  const [role, key] = call.label.split(":");
  if (role === "clarity") return { testObligations: [], gaps: [] };
  if (role === "impl") return built(`${key}.txt`)(call, serial);
  if (role === "review") return reviewed("pass")(call);
  if (role === "council") return vote(0);
  throw new Error(`no answer for the agent ${call.label}`);
};

// The runtime's calls, with the agent fake behind them. Couriers run at once,
// as the runtime's agents do, or with `serial`, one at a time. The fake
// agents' own git commands go one at a time (serial), and in that line with
// the couriers when those are too.
function runtime(dirs, roles, fault, concurrent) {
  const calls = [];
  const logs = [];
  const phases = [];
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
    if (String(opts.label).startsWith("courier:")) return concurrent ? carryOut(dirs, prompt, fault(call)) : serial(() => carryOut(dirs, prompt, fault(call)));
    const queue = queues.get(opts.label);
    const answer = queue && queue.length ? queue.shift() : byDefault;
    return typeof answer === "function" ? answer(call, serial) : answer;
  };
  // A barrier: every thunk runs, and one that throws gives null.
  const parallel = (thunks) => Promise.all(thunks.map((thunk) => Promise.resolve().then(thunk).catch(() => null)));
  // No barrier: each item goes through the stages on its own, and a stage that
  // throws drops it to null.
  const pipeline = (items, ...stages) => Promise.all(items.map((item, index) =>
    stages.reduce((prev, stage) => prev.then((value) => stage(value, item, index)), Promise.resolve(item)).catch(() => null)));
  return { calls, logs, phases, agent, parallel, pipeline, phase: (title) => phases.push(title), log: (message) => logs.push(message) };
}

// Runs the workflow to its end under the fake runtime, with the worktree's
// helper; gives what it returned or threw, and the runtime's record.
async function deliver(dirs, args = {}, roles = {}, { fault = () => null, source = SOURCE, concurrent = true } = {}) {
  const rt = runtime(dirs, roles, fault, concurrent);
  let result = null;
  let error = null;
  try {
    result = await compile(source)({ feature: "board", helper, ...args }, rt.agent, rt.parallel, rt.pipeline, rt.phase, rt.log, NoDate, NoRandom);
  } catch (e) {
    error = e;
  }
  return { result, error, calls: rt.calls, logs: rt.logs, phases: rt.phases };
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
  assert.deepEqual(couriers.map((c) => c.label), ["courier:start", "courier:unblock-pass", "courier:clarity-pending", "courier:list"]);
  for (const c of couriers) {
    assert.deepEqual([c.model, c.effort, c.agentType, c.schema.required], ["haiku", "low", undefined, ["exitCode", "output"]]);
    assert.match(c.prompt, /\. Run the helper\. Use the Bash tool, with its timeout set to 600000, to run this command exactly as written, once:\n/);
  }
  const startHash = fnv1a(JSON.stringify({ opId: "start-1", feature: "Bob's board", force: true }));
  assert.deepEqual([fileIn(couriers[0].prompt), commandIn(couriers[0].prompt)], [null, `node ${helper} start --op start-1 --feature 'Bob'\\''s board' --force --hash ${startHash}`]);
  const file = join(dirs.repo, ".omni", "runs", result.run, "in", "unblock-pass-1.json");
  const payload = JSON.stringify({ opId: "unblock-pass-1" });
  assert.deepEqual([fileIn(couriers[1].prompt), payloadIn(couriers[1].prompt), commandIn(couriers[1].prompt)], [file, payload, `node ${helper} unblock-pass --run ${result.run} --payload ${file} --hash ${fnv1a(payload)}`]);
  assert.deepEqual([result.feature, result.branch, JSON.parse(readFileSync(join(dirs.repo, ".omni", "run.lock"), "utf8")).feature], ["Bob's board", "deliver/bob-s-board", "Bob's board"]);
  assert.deepEqual(phases, ["Start", "Unblock", "Clarity", "Deliver"]);
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
  assert.equal(claims.length, 5, "four tries of the first claim, then the claim for review");
  assert.equal(new Set(claims.slice(0, 4).map((c) => c.prompt)).size, 1);
  assert.equal(fileIn(claims[0].prompt), join(dirs.repo, ".omni", "runs", result.run, "in", "claim-US-1-1.json"));
  assert.deepEqual(claimsOf(dirs, "US-1"), ["implementing", "review"]);
  const retries = logs.filter((l) => l.startsWith("courier:claim:US-1: "));
  assert.equal(retries.length, 3);
  assert.equal(retries[0], "courier:claim:US-1: the courier returned nothing; trying again (1 of 3)");
  assert.match(retries[1], /^courier:claim:US-1: the helper received another payload than the script sent: the payload hashes to [0-9a-f]{8}, not [0-9a-f]{8}: write it again exactly; trying again \(2 of 3\)$/);
  assert.equal(retries[2], "courier:claim:US-1: its output does not match its hash: the courier changed it; trying again (3 of 3)");
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "reviewed" }]);
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
    "clarity:US-1": [{ testObligations: ["A regression test covers the badge"], gaps }],
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
  assert.deepEqual(result.council, { settled: 3, asked: 0 });
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
  assert.deepEqual(result.council, { settled: 2, asked: 3 });
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
  assert.deepEqual((await helperStep(dirs, result.run, "clarity-pending")).keys, ["US-1"]);
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
  assert.deepEqual(result.council, { settled: 4, asked: 0 });
});

// ---- deliver: implement and review ---------------------------------------------------------

test("a story is claimed, gets its worktree and an implementer on sonnet, records its files, moves to implemented, is claimed for review, and a reviewer on opus meeting every criterion moves it to reviewed, where it waits for the lane", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }, { text: "It looks right", human_check: true }] })]);
  const roles = { "impl:US-1": [built("src/badge.rs")] };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(timeline(dirs, "US-1"), ["claim implementing", "move implemented", "claim review", "move reviewed"]);
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.blocked, done.files_changed], ["reviewed", null, false, ["src/badge.rs"]]);
  assert.deepEqual([agentsOf(calls, "impl:US-1").map((c) => c.model), agentsOf(calls, "review:US-1").map((c) => c.model)], [["sonnet"], ["opus"]]);
  assert.match(agentsOf(calls, "impl:US-1")[0].prompt, new RegExp(`\n## Your worktree\n${dirs.worktree("US-1")} \\(branch story/US-1, cut from deliver/board\\)`));
  assert.equal(git(dirs.worktree("US-1"), "rev-parse", "--abbrev-ref", "HEAD").trim(), "story/US-1");
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "reviewed" }]);
  assert.equal(git(dirs.repo, "rev-parse", "--abbrev-ref", "HEAD").trim(), "main");
});

test("an implementer that returns nothing has its story released with no attempt used; one that is blocked opens a thread with its reason, which blocks the story; one that is ready has its decisions recorded as the agent's", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);
  const roles = {
    "impl:US-1": [null],
    "impl:US-2": [{ status: "blocked", summary: "The badge API is missing from the spec." }],
    "impl:US-3": [built("src/badge.rs", { decisions: [{ question: "Which file holds the badge?", choice: "src/badge.rs" }] })],
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const outcome = (key) => result.outcomes.find((o) => o.key === key);
  assert.deepEqual(outcome("US-1"), { key: "US-1", outcome: "no_result", detail: "the implementer returned nothing; the story is released without using an attempt" });
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").holder, dirs.story("US-1").blocked, dirs.story("US-1").findings || []], ["backlog", null, false, []]);
  const [blocked] = questions(dirs);
  assert.deepEqual([blocked.title, blocked.posts[0].body, outcome("US-2")], ["US-2: the implementer is blocked", "The badge API is missing from the spec.\n", { key: "US-2", outcome: "blocked", thread: blocked.id }]);
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").blocked, dirs.story("US-2").links], ["backlog", true, [{ thread_id: blocked.id, label: "question" }]]);
  assert.deepEqual(dirs.story("US-3").decisions, [{ question: "Which file holds the badge?", choice: "src/badge.rs", by: "agent" }]);
  assert.equal(dirs.story("US-3").status, "reviewed");
});

test("a story whose delivery throws is released and gets a thread with the error, which blocks it, so the human hears and no run cycles on it: here an implementer left its worktree on another branch", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }] })]);
  const roles = {
    "impl:US-1": [async (call, serial) => {
      await serial(() => {
        const wt = worktreeIn(call.prompt);
        commit(wt, "src/badge.rs");
        git(wt, "checkout", "-q", "-b", "side");
      });
      return { status: "ready", summary: "Built.", decisions: [], commits: [] };
    }],
    "review:US-1": [reviewed("fail", { unmet: { 0: "Pink" } })],
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const [stopped] = questions(dirs);
  assert.equal(stopped.title, "US-1: delivery stopped on an error");
  assert.match(stopped.posts[0].body, /^The workflow stopped delivering US-1 on this error:\n\ncourier:worktree:US-1: worktree failed: \S+ is not a worktree of \S+ on story\/US-1; move it away, after 3 retries\n\nThe story is back in its waiting state, and blocked until this thread is answered; answer it once the cause is fixed\.\n$/);
  const done = dirs.story("US-1");
  assert.deepEqual([done.status, done.holder, done.blocked, done.links], ["backlog", null, true, [{ thread_id: stopped.id, label: "question" }]]);
  assert.deepEqual(result.outcomes.map((o) => [o.key, o.outcome, o.thread]), [["US-1", "failed", stopped.id]]);
});

test("a story waiting in implemented starts at its review; a criterion that names a test is a test obligation the review never judges, whether clarity recorded it or the reviewer names it", { timeout: 60000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "A regression test covers the badge", human_check: false }];
  const dirs = scratch(t, [story("US-1", { status: "implemented", criteria }), story("US-2", { criteria })]);
  git(dirs.repo, "worktree", "add", "-q", "-b", "story/US-1", dirs.worktree("US-1"));
  commit(dirs.worktree("US-1"), "src/badge.rs");
  const roles = {
    // US-1 came before clarity, so no decision: its reviewer names the obligation.
    "review:US-1": [reviewed("pass", { unmet: { 1: "No test in the diff" }, obligations: [1] })],
    "clarity:US-2": [{ testObligations: ["A regression test covers the badge"], gaps: [] }],
    // US-2's reviewer judges the obligation clarity recorded; the judgement is not read.
    "review:US-2": [reviewed("pass", { unmet: { 1: "No test in the diff" } })],
  };

  const { result, error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual([timeline(dirs, "US-1"), dirs.story("US-1").status], [["claim review", "move reviewed"], "reviewed"]);
  assert.deepEqual(agentsOf(calls, "impl:US-1"), []);
  assert.equal(dirs.story("US-2").status, "reviewed");
  assert.match(agentsOf(calls, "review:US-2")[0].prompt, /\n0: The badge is red\n1: A regression test covers the badge \[test obligation\]\n/);
  assert.match(agentsOf(calls, "review:US-1")[0].prompt, /\n0: The badge is red\n1: A regression test covers the badge\n/);
  assert.deepEqual([dirs.story("US-1").findings || [], dirs.story("US-2").findings || []], [[], []]);
  assert.deepEqual(result.outcomes.map((o) => o.outcome), ["reviewed", "reviewed"]);
});

test("criteria are numbered from 0 in the review's schema, which wants an entry for each; a review that leaves one without a verdict is malformed and asked again, never taken as met, and a story whose reviewer stays malformed is released with no attempt used", { timeout: 60000 }, async (t) => {
  const criteria = [{ text: "The badge is red", human_check: false }, { text: "The badge counts", human_check: false }];
  const dirs = scratch(t, [story("US-1", { criteria }), story("US-2", { criteria })]);
  // Numbered from 1: the entry for 0 is missing, and the one for 2 is past the end.
  const fromOne = (call) => ({ verdict: "pass", criteria: criteriaIn(call.prompt).map((c) => ({ ...c, index: c.index + 1, obligation: false, met: true, evidence: "Seen" })), defects: [] });
  const roles = { "review:US-1": [fromOne, reviewed("pass")], "review:US-2": [fromOne, fromOne, fromOne] };

  const { result, error, calls, logs } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const schema = agentsOf(calls, "review:US-1")[0].schema.properties.criteria;
  assert.deepEqual([schema.minItems, schema.maxItems, schema.items.properties.index.minimum, schema.items.properties.index.maximum], [2, 2, 0, 1]);
  assert.match(schema.items.properties.index.description, /counting from 0/);
  assert.deepEqual([agentsOf(calls, "review:US-1").length, dirs.story("US-1").status], [2, "reviewed"]);
  assert.ok(logs.includes("US-1: the review gave no verdict on criteria 0; asking again"), logs.join("\n"));
  assert.equal(agentsOf(calls, "review:US-2").length, 3);
  assert.deepEqual(result.outcomes.find((o) => o.key === "US-2"), { key: "US-2", outcome: "no_result", detail: "the reviewer gave no verdict on criteria 0, 3 times; the story is released without using an attempt" });
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").holder, dirs.story("US-2").findings || []], ["implemented", null, []]);
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

test("a review fails on a criterion not met, even with the verdict pass, and on every defect it lists: each becomes a review finding of the attempt, and the story goes back for the next attempt until a review meets every criterion with no defect", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }, { text: "The badge counts", human_check: false }] })]);
  const roles = {
    "review:US-1": [
      reviewed("pass", { unmet: { 1: "No count at src/badge.rs:3" } }),
      reviewed("fail", { defects: [{ text: "A new test file in the diff", files: ["badge.test.rs"], criterion: "" }, { text: "The count is off by one", files: ["src/badge.rs"], criterion: "The badge counts" }] }),
      reviewed("pass"),
    ],
  };

  const { result, error } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  assert.deepEqual(findingsOf(dirs, "US-1"), [
    ["review", '[attempt 1] Criterion not met: "The badge counts". No count at src/badge.rs:3', [], false],
    ["review", "[attempt 2] A new test file in the diff", ["badge.test.rs"], false],
    ["review", '[attempt 2] The count is off by one (criterion: "The badge counts")', ["src/badge.rs"], false],
  ]);
  assert.deepEqual(timeline(dirs, "US-1"), [
    "claim implementing", "move implemented", "claim review", "move backlog",
    "claim implementing", "move implemented", "claim review", "move backlog",
    "claim implementing", "move implemented", "claim review", "move reviewed",
  ]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "reviewed" }]);
});

test("from attempt 2 the implementer runs on implRetry, in the same worktree, and its prompt lists every finding since the human's latest answer by attempt, those that used no attempt, and the earlier ones apart", { timeout: 60000 }, async (t) => {
  const answered = { stage: "testing", text: "[attempt 1] The badge was blue", files: [], resolved: true };
  const conflict = { stage: "testing", text: "rebase conflict in README: rebase it onto deliver/board", files: ["README"], resolved: false };
  const dirs = scratch(t, [story("US-1", { criteria: [{ text: "The badge is red", human_check: false }], findings: [answered, conflict] })]);
  const defect = (text, file) => reviewed("fail", { defects: [{ text, files: [file], criterion: "" }] });
  const roles = { "review:US-1": [defect("The badge is pink", "src/badge.rs"), defect("The badge has no border", "src/border.rs"), reviewed("pass")] };

  const { error, calls } = await deliver(dirs, {}, roles);

  assert.equal(error, null);
  const impls = agentsOf(calls, "impl:US-1");
  assert.deepEqual(impls.map((c) => c.model), ["sonnet", "opus", "opus"]);
  assert.deepEqual(impls.map((c) => worktreeIn(c.prompt)), [1, 2, 3].map(() => dirs.worktree("US-1")));
  assert.deepEqual(impls.map((c) => lineAfter(c.prompt, "## Your task now\n")), [
    "This is attempt 1 of 3 since the human's latest answer. Implement the story: build what the criteria above require, commit it, and return the result.",
    "This is attempt 2 of 3 since the human's latest answer. A worktree of an earlier attempt is in place, with its commits and what it left uncommitted: continue from it, do not start over, and fix every finding above.",
    "This is attempt 3 of 3 since the human's latest answer. A worktree of an earlier attempt is in place, with its commits and what it left uncommitted: continue from it, do not start over, and fix every finding above.",
  ]);
  const earlier = "\n### Findings before the human's latest answer (what failed before it)\nAttempt 1:\n- [testing] The badge was blue\n";
  const uncounted = "\n### Other open findings (fix every one of them; they used no attempt)\n- [testing] rebase conflict in README: rebase it onto deliver/board (README)\n";
  assert.ok(impls[0].prompt.includes(`${uncounted}${earlier}`), impls[0].prompt);
  assert.doesNotMatch(impls[0].prompt, /Findings of the earlier attempts/);
  assert.ok(impls[2].prompt.includes(`\n### Findings of the earlier attempts (fix every one of them)
Attempt 1:
- [review] The badge is pink (src/badge.rs)
Attempt 2:
- [review] The badge has no border (src/border.rs)
${uncounted}${earlier}`), impls[2].prompt);
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
  assert.match(impls[1].prompt, /\n- Which red\? -> #c00 \(council: "const RED = '#c00'" \(src\/colors\.rs:3\)\) \(agent\)\n/);
  assert.match(lineAfter(impls[1].prompt, "## Your task now\n"), /An earlier session of this attempt stopped to ask a question, which the decisions above now settle\./);
  for (const c of agentsOf(calls, "council:US-1:code")) assert.ok(c.prompt.includes(`You read the code, and nothing else: the story's worktree ${dirs.worktree("US-1")}.`), c.prompt);
  assert.equal(agentsOf(calls, "council:US-1:code").length, 2);
  const [blinking] = questions(dirs);
  assert.deepEqual([questions(dirs).length, blinking.title], [1, "US-1: Should the badge blink?"]);
  assert.match(blinking.posts[0].body, /\nAsked by the reviewer\.\n[\s\S]*\nRecommendation: No \(the council's majority\)\n$/);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").blocked], ["implemented", true]);
  assert.deepEqual(result.outcomes, [{ key: "US-1", outcome: "needs_decision", thread: blinking.id }]);
  assert.deepEqual(result.council, { settled: 1, asked: 1 });
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
  assert.deepEqual(result.council, { settled: 3, asked: 1 });
});

test("an implementer's slot is free once its story is implemented: with implementers 3, a fourth story starts implementing while an earlier one waits in review", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  let fourthStarted;
  const fourth = new Promise((resolve) => { fourthStarted = resolve; });
  // The first three stories' reviewers answer once the fourth story's
  // implementer has started, or after 5 s if it never does: with their slots
  // held through review, it would not start before one of them ended.
  const waiting = async (call) => {
    await within(fourth, 5000);
    return reviewed("pass")(call);
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
  assert.deepEqual(keys.map((key) => dirs.story(key).status), ["reviewed", "reviewed", "reviewed", "reviewed"]);
  assert.deepEqual(result.outcomes.map((o) => o.key).sort(), keys);
});

test("at most `implementers` stories implement at once and at most `reviewers` are in review", { timeout: 60000 }, async (t) => {
  const keys = ["US-1", "US-2", "US-3", "US-4"];
  const dirs = scratch(t, keys.map((key) => story(key)));
  // A review takes half a second, long enough for the next story's to start beside it.
  const slow = async (call) => {
    await pause(500);
    return reviewed("pass")(call);
  };

  const { result, error } = await deliver(dirs, { implementers: 2, reviewers: 1 }, Object.fromEntries(keys.map((key) => [`review:${key}`, [slow]])));

  assert.equal(error, null);
  assert.deepEqual([peakIn(dirs, "implementing", ["implemented"]), peakIn(dirs, "review", ["reviewed", "backlog"])], [2, 1]);
  assert.deepEqual(keys.map((key) => dirs.story(key).status), ["reviewed", "reviewed", "reviewed", "reviewed"]);
  assert.deepEqual(result.outcomes.map((o) => o.key).sort(), keys);
});

test("a story its review sends back takes a new implementer slot: with implementers 1, the next story implements while the first is in review, and the first waits for the slot to come back", { timeout: 60000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  // The second story's implementer works on until the first is back in the backlog with its finding.
  const back = (serial) => serial(() => dirs.story("US-1").status === "backlog" && (dirs.story("US-1").findings || []).length > 0);
  const roles = {
    "review:US-1": [reviewed("fail", { defects: [{ text: "The badge is pink", files: ["src/badge.rs"], criterion: "" }] }), reviewed("pass")],
    "impl:US-2": [async (call, serial) => {
      for (let i = 0; i < 100 && !(await back(serial)); i++) await pause(50);
      return built("US-2.txt")(call, serial);
    }],
  };

  const { error } = await deliver(dirs, { implementers: 1 }, roles);

  assert.equal(error, null);
  const order = boardLog(dirs);
  assert.deepEqual(order.filter((e) => e.startsWith("claim US-1 ") || e.startsWith("move US-1 ")), [
    "claim US-1 implementing", "move US-1 implemented", "claim US-1 review", "move US-1 backlog",
    "claim US-1 implementing", "move US-1 implemented", "claim US-1 review", "move US-1 reviewed",
  ]);
  assert.ok(order.indexOf("claim US-2 implementing") < order.indexOf("move US-1 backlog"), order.join("\n"));
  assert.ok(order.indexOf("move US-2 implemented") < order.lastIndexOf("claim US-1 implementing"), order.join("\n"));
  assert.equal(peakIn(dirs, "implementing", ["implemented"]), 1);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-2").status], ["reviewed", "reviewed"]);
});

// The script with its agent cap set lower; the real values are pinned first.
const capped = (values) => {
  let source = SOURCE;
  for (const [name, value] of Object.entries(values)) {
    const real = { AGENT_CAP: 900, HARD_CAP: 960, STORY_CALLS: 60, CHECK_CALLS: 20 }[name];
    assert.ok(source.includes(`\nconst ${name} = ${real}\n`), `${name} is ${real}`);
    source = source.replace(`\nconst ${name} = ${real}\n`, `\nconst ${name} = ${value}\n`);
  }
  return source;
};

test("the run counts every agent call, couriers too, and takes a new story only while the calls made and those set aside for the stories in flight, 60 each with the new one, stay within 900", { timeout: 60000 }, async (t) => {
  // 12 calls come before the first claim. With 60 and 15 for 900 and 60, the
  // first two stories fit; the third would be the third in flight, past 60.
  const source = capped({ AGENT_CAP: 60, STORY_CALLS: 15 });
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);

  const { result, error, logs } = await deliver(dirs, { implementers: 2 }, {}, { source });

  assert.equal(error, null);
  assert.deepEqual(["US-1", "US-2", "US-3"].map((key) => claimsOf(dirs, key)), [["implementing", "review"], ["implementing", "review"], []]);
  assert.deepEqual(["US-1", "US-2", "US-3"].map((key) => dirs.story(key).status), ["reviewed", "reviewed", "backlog"]);
  assert.equal(result.capped, true);
  assert.ok(logs.some((l) => /^The run has made \d+ agent calls, with 2 stories in flight: it takes no new story, and ends once those are done; the next run carries on$/.test(l)), logs.join("\n"));
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
  assert.deepEqual((await helperStep(dirs, result.run, "clarity-pending")).keys, ["US-4", "US-5"]);
  assert.deepEqual([result.capped, keys.flatMap((key) => claimsOf(dirs, key))], [true, []]);
});

test("prompts give the context first and the criteria and the task last, name the linked docs and nodes for the agent to read once with omni as this run's role, use absolute paths, say who is read-only and that the workflow makes every board write; models and agent types come from args", { timeout: 60000 }, async (t) => {
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
  assert.deepEqual([clarity, impl, review].map((c) => [c.model, c.agentType]), [["opus", undefined], ["sonnet", "implementer"], ["sonnet", "verifier"]]);
  assert.equal(result.checks.stack, "make stack-tests");
  const order = (prompt, headings) => headings.map((h) => prompt.indexOf(`\n${h}\n`));
  for (const [prompt, headings] of [
    [clarity.prompt, ["## The story", "## The shared parts", "## The board", "## Acceptance criteria", "## Your task now"]],
    [impl.prompt, ["## Your worktree", "## The story", "## The linked docs", "## The board", "## Rules", "## Acceptance criteria", "## Your task now"]],
    [review.prompt, ["## The change", "## The story", "## The linked docs", "## The board", "## Rules", "## Acceptance criteria (number: text)", "## Your task now"]],
  ]) {
    const at = order(prompt, headings);
    assert.ok(at.every((n) => n > 0), `${headings} in ${prompt}`);
    assert.deepEqual(at, [...at].sort((a, b) => a - b), prompt);
    assert.match(prompt, /\nThe workflow makes every write to the board itself, through its helper, from what you return\./);
    assert.doesNotMatch(prompt, /SPEC NODE/);
  }
  const nodes = "- doc doc-spec: node n1 (spec), node n2 (counts)";
  assert.ok(impl.prompt.includes(`Read each doc below once, with \`OMNI_RUN=${result.run} OMNI_ROLE=impl OMNI_FEATURE='board' omni doc read <doc>\`, and find in it the nodes the story links: each is the element with that data-omni-id.\n${nodes}`), impl.prompt);
  assert.ok(review.prompt.includes(`OMNI_RUN=${result.run} OMNI_ROLE=review OMNI_FEATURE='board' omni doc read <doc>`));
  assert.ok(clarity.prompt.includes(`\`OMNI_RUN=${result.run} OMNI_ROLE=clarity OMNI_FEATURE='board' omni doc list\``));
  assert.ok(clarity.prompt.includes(nodes));
  assert.match(clarity.prompt, /\n## Acceptance criteria\n- The badge is red\n- It looks right \[human check\]\n/);
  assert.match(impl.prompt, /\n- Which red\? -> #c00 \(human\)\n/);
  assert.match(impl.prompt, /Never start the stack or run the stack tests \(`make stack-tests`\)/);
  assert.ok(review.prompt.includes(`See the diff with \`git -C ${dirs.worktree("US-1")} diff refs/heads/deliver/board...HEAD\``));
  assert.match(review.prompt, /\n## Acceptance criteria \(number: text\)\n0: The badge is red\n1: It looks right \[human check\]\n/);
  for (const prompt of [clarity.prompt, review.prompt]) assert.match(prompt, /You are read-only: you change no file, make no commit, and write nothing to the board\./);
  assert.doesNotMatch(impl.prompt, /You are read-only/);
});
