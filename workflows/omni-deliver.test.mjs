// omni-deliver's run start, run in-process the way the workflow runtime runs it
// (args, agent, phase, parallel, log), against a real temp git repository and a
// fake omni first on PATH (fixtures/fake-omni.mjs) whose state is a JSON file.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const source = readFileSync(fileURLToPath(new URL("./omni-deliver.js", import.meta.url)), "utf8").replace("export const meta", "const meta");
const fakeOmni = fileURLToPath(new URL("./fixtures/fake-omni.mjs", import.meta.url));
const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
const workflow = new AsyncFunction("args", "agent", "phase", "parallel", "log", source);

const git = (cwd, ...argv) => execFileSync("git", argv, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });

function scratch(t, stories = [], threads = []) {
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
  writeFileSync(statePath, JSON.stringify({ stories, threads, calls: [] }));

  const saved = { cwd: process.cwd(), PATH: process.env.PATH, state: process.env.FAKE_OMNI_STATE };
  process.chdir(repo);
  process.env.PATH = `${bin}:${saved.PATH}`;
  process.env.FAKE_OMNI_STATE = statePath;
  t.after(() => {
    process.chdir(saved.cwd);
    process.env.PATH = saved.PATH;
    if (saved.state === undefined) delete process.env.FAKE_OMNI_STATE;
    else process.env.FAKE_OMNI_STATE = saved.state;
    delete process.env.FAKE_OMNI_FAIL;
    rmSync(root, { recursive: true, force: true });
  });
  return {
    root,
    repo,
    lock: join(repo, ".omni", "run.lock"),
    state: () => JSON.parse(readFileSync(statePath, "utf8")),
    checks: (checks) => writeFileSync(statePath, JSON.stringify({ ...JSON.parse(readFileSync(statePath, "utf8")), checks })),
    story: (key) => JSON.parse(readFileSync(statePath, "utf8")).stories.find((s) => s.key === key),
  };
}

function story(key, fields = {}) {
  return { key, title: `Story ${key}`, text: "", criteria: [], status: "backlog", blocked: false, holder: null, version: 1, decisions: [], links: [], blocked_by: [], files_planned: [], ...fields };
}

// The story's worktree beside the checkout, as the run lays them out.
function worktree(dirs, key) {
  const path = join(dirs.root, "app-stories", key);
  git(dirs.repo, "worktree", "add", "-q", "-b", `story/${key}`, path);
  return path;
}

// A clarity agent that answers with the gaps given per story key.
function clarityAgent(gapsByKey = {}) {
  const prompts = [];
  const agent = async (prompt) => {
    prompts.push(prompt);
    const key = Object.keys(gapsByKey).find((k) => prompt.includes(`"${k}"`));
    return { gaps: key ? gapsByKey[key] : [] };
  };
  return { agent, prompts };
}

function start(feature, agent = clarityAgent().agent) {
  return workflow({ feature }, agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), () => {});
}

function liveProcess(t) {
  const child = spawn("sleep", ["60"], { stdio: "ignore" });
  t.after(() => child.kill());
  return child.pid;
}

function deadPid() {
  return spawnSync("true").pid;
}

function writeLock(dirs, lock) {
  mkdirSync(join(dirs.repo, ".omni"), { recursive: true });
  writeFileSync(dirs.lock, JSON.stringify(lock) + "\n");
}

// The claims this run sent for the story, in order.
const claimsOf = (dirs, key) => dirs.state().calls.filter((c) => c.argv[1] === "claim" && c.argv[2] === key).map((c) => ({ run: c.argv[c.argv.indexOf("--run") + 1], stage: c.argv[c.argv.indexOf("--stage") + 1] }));

const blockingGap = { criterion: "The badge has a colour", question: "Which colour is the badge?", options: ["Paint it red", "Paint it blue"], recommendation: "Paint it red", reason: "\"warning colour\"", blocking: true };
const minorGap = (n) => ({ criterion: `Criterion ${n}`, question: `Minor question ${n}?`, options: [`Option ${n}a`, `Option ${n}b`], recommendation: `Option ${n}a`, reason: "convention", blocking: false });

// ---- the fake omni's phase rules ----------------------------------------------

const omni = (...argv) => spawnSync("omni", argv, { encoding: "utf8" });

test("the fake refuses a claim from a status that is not its stage's waiting state, changing nothing, and claims from the waiting state", (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { status: "implemented" })]);

  const refused = omni("story", "claim", "US-1", "--run", "r", "--stage", "review");
  const claimed = omni("story", "claim", "US-2", "--run", "r", "--stage", "review");

  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /not_waiting/);
  assert.deepEqual(dirs.story("US-1"), story("US-1"));
  assert.equal(claimed.status, 0);
  assert.equal(dirs.story("US-2").status, "review");
  assert.deepEqual(dirs.story("US-2").holder, { run: "r", stage: "review" });
});

test("the fake's holder moving an implementing story to implemented clears the holder", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implementing", holder: { run: "r", stage: "implementing" } })]);

  const moved = omni("story", "move", "US-1", "--run", "r", "--to", "implemented");

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

  omni("story", "release", "US-1", "--run", "r");
  omni("story", "release", "US-2", "--run", "r");
  omni("story", "release", "US-3", "--run", "r");

  assert.deepEqual(dirs.state().stories.map((s) => [s.key, s.status, s.holder, s.blocked]), [["US-1", "backlog", null, false], ["US-2", "implemented", null, false], ["US-3", "reviewed", null, false]]);
});

test("the fake's block on a story held in testing returns it to reviewed with no holder and blocked", (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", holder: { run: "r", stage: "testing" } })]);

  omni("story", "block", "US-1", "--run", "r");

  assert.equal(dirs.story("US-1").status, "reviewed");
  assert.equal(dirs.story("US-1").holder, null);
  assert.equal(dirs.story("US-1").blocked, true);
});

test("the fake leaves out of pickable a backlog story whose planned file a reviewed story changed, and lists it once that story is done", (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "reviewed", files_planned: ["src/other.rs"], files_changed: ["src/board.rs"] }),
    story("US-2", { files_planned: ["src/board.rs"] }),
  ]);

  const whileReviewed = omni("story", "list", "--pickable").stdout;
  omni("story", "move", "US-1", "--run", "r", "--to", "done");
  const onceDone = omni("story", "list", "--pickable").stdout;

  assert.equal(whileReviewed, "");
  assert.equal(onceDone, "US-2\n");
});

// ---- start: clean tree and lock ---------------------------------------------

test("refuses a dirty main worktree with 'the tree is not clean' and writes no lock", async (t) => {
  const dirs = scratch(t);
  writeFileSync(join(dirs.repo, "README"), "changed\n");

  await assert.rejects(start("board"), { message: "the tree is not clean" });

  assert.equal(existsSync(dirs.lock), false);
  assert.deepEqual(dirs.state().calls, []);
});

test("reports the dirty tree, not a live lock, when both apply, and leaves the lock as it was", async (t) => {
  const dirs = scratch(t);
  const held = { run: "run-20261030-101010-1", pid: liveProcess(t), feature: "board", started: "2026-10-30T10:10:10Z" };
  writeLock(dirs, held);
  writeFileSync(join(dirs.repo, "README"), "changed\n");

  await assert.rejects(start("board"), { message: "the tree is not clean" });

  assert.deepEqual(JSON.parse(readFileSync(dirs.lock, "utf8")), held);
});

test("refuses a lock with a live pid, naming the holding run and its feature, even for another feature", async (t) => {
  const dirs = scratch(t);
  const held = { run: "run-20261030-101010-1", pid: liveProcess(t), feature: "billing", started: "2026-10-30T10:10:10Z" };
  writeLock(dirs, held);

  await assert.rejects(start("board"), { message: "the tree is held by run run-20261030-101010-1 for feature billing" });

  assert.deepEqual(JSON.parse(readFileSync(dirs.lock, "utf8")), held);
  assert.deepEqual(dirs.state().calls, []);
});

test("takes over a lock with a dead pid: the lock names this run and this process, and the thread Run <id> is created on the feature", async (t) => {
  const dirs = scratch(t);
  writeLock(dirs, { run: "run-20261030-101010-1", pid: deadPid(), feature: "billing", started: "2026-10-30T10:10:10Z" });

  await start("board");

  const create = dirs.state().calls.find((c) => c.argv[0] === "thread" && c.argv[1] === "create");
  assert.match(create.lock.run, new RegExp(`^run-\\d{8}-\\d{6}-${process.pid}$`));
  assert.equal(create.lock.pid, process.pid);
  assert.equal(create.lock.feature, "board");
  assert.deepEqual(dirs.state().threads.map((th) => [th.title, th.feature]), [[`Run ${create.lock.run}`, "board"]]);
});

test("removes its lock when the run fails after taking it", async (t) => {
  const dirs = scratch(t);
  process.env.FAKE_OMNI_FAIL = "thread";

  await assert.rejects(start("board"), /thread create failed/);

  assert.notEqual(dirs.state().calls.find((c) => c.argv[0] === "thread").lock, null);
  assert.equal(existsSync(dirs.lock), false);
});

test("names the run by local time", async (t) => {
  const dirs = scratch(t);
  const tz = process.env.TZ;
  process.env.TZ = "Asia/Tokyo";
  t.after(() => {
    if (tz === undefined) delete process.env.TZ;
    else process.env.TZ = tz;
  });
  t.mock.timers.enable({ apis: ["Date"], now: Date.UTC(2026, 9, 30, 19, 0, 0) });

  await start("board");

  const create = dirs.state().calls.find((c) => c.argv[0] === "thread" && c.argv[1] === "create");
  assert.equal(create.lock.run, `run-20261031-040000-${process.pid}`);
});

// ---- reconciliation ---------------------------------------------------------

// A story a dead run held: blocked with an open thread, so that once released it
// waits where the release put it and no loop of this run takes it further.
const deadHeld = (key, status, threadId) => story(key, { status, holder: { run: "run-old", stage: status }, blocked: true, links: [{ thread_id: threadId, label: "question" }] });
const openThread = (id, key) => ({ id, title: `${key}: Which colour is the badge?`, feature: "board", posts: [], conclusion: null });

// Two commits on the integration branch past the one returned, as a merge leaves them.
function twoCommitsPast(dirs) {
  const base = git(dirs.repo, "rev-parse", "HEAD").trim();
  for (const n of [1, 2]) {
    writeFileSync(join(dirs.repo, `merged${n}`), "merged\n");
    git(dirs.repo, "add", `merged${n}`);
    git(dirs.repo, "commit", "-q", "-m", `merged ${n}`);
  }
  return base;
}

function writeLane(dirs, text) {
  mkdirSync(join(dirs.repo, ".omni"), { recursive: true });
  writeFileSync(join(dirs.repo, ".omni", "lane.json"), text);
}

const laneFile = (dirs) => join(dirs.repo, ".omni", "lane.json");
const releasedSection = (report) => report.split("## Released from a dead run\n")[1].split("\n\n")[0].split("\n");

test("a dead run's stories in review, testing and implementing wait unheld in implemented, reviewed and backlog, the implementing one's worktree kept", async (t) => {
  const dirs = scratch(t, [deadHeld("US-1", "review", "th-1"), deadHeld("US-2", "testing", "th-2"), deadHeld("US-3", "implementing", "th-3")], [openThread("th-1", "US-1"), openThread("th-2", "US-2"), openThread("th-3", "US-3")]);
  const wt = worktree(dirs, "US-3");
  writeFileSync(join(wt, "work"), "work\n");
  git(wt, "add", "work");
  git(wt, "commit", "-q", "-m", "work");

  await start("board");

  assert.deepEqual(["US-1", "US-2", "US-3"].map((k) => [dirs.story(k).status, dirs.story(k).holder]), [["implemented", null], ["reviewed", null], ["backlog", null]]);
  assert.equal(readFileSync(join(wt, "work"), "utf8"), "work\n");
  assert.deepEqual(["US-1", "US-2", "US-3"].map((k) => claimsOf(dirs, k)), [[], [], []]);
});

test("a lane file left by a dead run resets the integration branch to its base, releases its story to reviewed and is removed; the report lists that story once beside the others released", async (t) => {
  const dirs = scratch(t, [deadHeld("US-1", "testing", "th-1"), deadHeld("US-2", "review", "th-2")], [openThread("th-1", "US-1"), openThread("th-2", "US-2")]);
  const base = twoCommitsPast(dirs);
  writeLane(dirs, JSON.stringify({ key: "US-1", base }));

  const result = await start("board");

  assert.equal(git(dirs.repo, "rev-parse", "main").trim(), base);
  assert.equal(git(dirs.repo, "status", "--porcelain"), "");
  assert.equal(dirs.story("US-1").status, "reviewed");
  assert.equal(dirs.story("US-1").holder, null);
  assert.equal(existsSync(laneFile(dirs)), false);
  assert.deepEqual(releasedSection(result.report).sort(), ["- US-1: reviewed", "- US-2: implemented"]);
});

test("without a lane file the integration branch is untouched and, nothing being released, the report has no released section", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", blocked: true, links: [{ thread_id: "th-1", label: "question" }] })], [openThread("th-1", "US-1")]);
  twoCommitsPast(dirs);
  const tip = git(dirs.repo, "rev-parse", "HEAD").trim();

  const result = await start("board");

  assert.equal(git(dirs.repo, "rev-parse", "main").trim(), tip);
  assert.equal(result.report.includes("Released from a dead run"), false);
});

test("a lane file naming a done story is removed without a reset", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "done" })]);
  const base = twoCommitsPast(dirs);
  const tip = git(dirs.repo, "rev-parse", "HEAD").trim();
  writeLane(dirs, JSON.stringify({ key: "US-1", base }));

  await start("board");

  assert.equal(git(dirs.repo, "rev-parse", "main").trim(), tip);
  assert.equal(existsSync(laneFile(dirs)), false);
  assert.equal(dirs.story("US-1").status, "done");
});

test("a lane file naming a cancelled story resets the branch to its base and leaves the story cancelled", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "cancelled" })]);
  const base = twoCommitsPast(dirs);
  writeLane(dirs, JSON.stringify({ key: "US-1", base }));

  await start("board");

  assert.equal(git(dirs.repo, "rev-parse", "main").trim(), base);
  assert.equal(existsSync(laneFile(dirs)), false);
  assert.equal(dirs.story("US-1").status, "cancelled");
});

test("a lane file that cannot be parsed is removed without a reset", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "done" })]);
  twoCommitsPast(dirs);
  const tip = git(dirs.repo, "rev-parse", "HEAD").trim();
  writeLane(dirs, "{not json");

  await start("board");

  assert.equal(git(dirs.repo, "rev-parse", "main").trim(), tip);
  assert.equal(existsSync(laneFile(dirs)), false);
});

test("returns an implementing story whose worktree is missing to backlog unheld", async (t) => {
  const dirs = scratch(t, [story("US-2", { status: "implementing", holder: { run: "run-old", stage: "implementing" } })]);

  await start("board");

  assert.equal(dirs.story("US-2").status, "backlog");
  assert.equal(dirs.story("US-2").holder, null);
});

// ---- clarity ----------------------------------------------------------------

test("a blocking gap becomes a thread 'US-<n>: <question>' with the options and recommendation, linked from the story, and blocks it", async (t) => {
  const dirs = scratch(t, [story("US-7", { links: [{ doc_id: "spec", node_id: "n1", label: "spec" }] })]);

  await start("board", clarityAgent({ "US-7": [blockingGap] }).agent);

  const thread = dirs.state().threads.find((th) => th.title === "US-7: Which colour is the badge?");
  assert.equal(thread.feature, "board");
  assert.equal(thread.posts.length, 1);
  assert.ok(thread.posts[0].body.includes("Paint it red"));
  assert.ok(thread.posts[0].body.includes("Paint it blue"));
  assert.match(thread.posts[0].body, /recommend[^\n]*Paint it red/i);
  assert.deepEqual(dirs.story("US-7").links, [{ doc_id: "spec", node_id: "n1", label: "spec" }, { thread_id: thread.id, label: "question" }]);
  assert.equal(dirs.story("US-7").blocked, true);
  assert.deepEqual(dirs.story("US-7").decisions, []);
});

test("each minor gap becomes an agent decision and the story stays unblocked in backlog", async (t) => {
  const dirs = scratch(t, [story("US-8")]);

  await start("board", clarityAgent({ "US-8": [minorGap(1), minorGap(2)] }).agent);

  assert.deepEqual(dirs.story("US-8").decisions, [
    { question: "Minor question 1?", choice: "Option 1a", by: "agent" },
    { question: "Minor question 2?", choice: "Option 2a", by: "agent" },
  ]);
  assert.equal(dirs.story("US-8").blocked, false);
  assert.equal(dirs.story("US-8").status, "backlog");
  assert.deepEqual(dirs.state().threads.map((th) => th.title).filter((title) => title.startsWith("US-")), []);
});

test("records each checked story's version after its writes, so a second start checks neither again", async (t) => {
  const dirs = scratch(t, [story("US-7"), story("US-8")]);
  await start("board", clarityAgent({ "US-7": [blockingGap], "US-8": [minorGap(1)] }).agent);

  const recorded = JSON.parse(readFileSync(join(dirs.repo, ".omni", "clarity.json"), "utf8"));
  assert.deepEqual(recorded, { "US-7": dirs.story("US-7").version, "US-8": dirs.story("US-8").version });

  const second = clarityAgent({ "US-7": [blockingGap], "US-8": [minorGap(1)] });
  await start("board", second.agent);
  assert.deepEqual(second.prompts, []);
});

test("checks again a backlog story whose version differs from the recorded one", async (t) => {
  const dirs = scratch(t, [story("US-8", { version: 5 })]);
  mkdirSync(join(dirs.repo, ".omni"));
  writeFileSync(join(dirs.repo, ".omni", "clarity.json"), JSON.stringify({ "US-8": 4 }));
  const clarity = clarityAgent({ "US-8": [minorGap(1)] });

  await start("board", clarity.agent);

  assert.equal(clarity.prompts.length, 1);
  assert.deepEqual(dirs.story("US-8").decisions, [{ question: "Minor question 1?", choice: "Option 1a", by: "agent" }]);
});

test("does not check a blocked backlog story", async (t) => {
  scratch(t, [story("US-9", { blocked: true })]);
  const clarity = clarityAgent();

  await start("board", clarity.agent);

  assert.deepEqual(clarity.prompts, []);
});

test("gives the clarity check the story's recorded decisions", async (t) => {
  scratch(t, [story("US-8", { decisions: [{ question: "Which badge size?", choice: "Small badges", by: "human" }] })]);
  const clarity = clarityAgent();

  await start("board", clarity.agent);

  assert.ok(clarity.prompts[0].includes("Which badge size?"));
  assert.ok(clarity.prompts[0].includes("Small badges"));
});

// ---- unblock ----------------------------------------------------------------

// A thread on the feature as `omni thread read` shows it; conclusion null when open.
function thread(id, title, conclusion = null) {
  return { id, title, feature: "board", posts: [], conclusion };
}

const questionLink = (id) => ({ thread_id: id, label: "question" });

test("unblocks a blocked backlog story whose one thread has a conclusion, recording it as a human decision with the thread's question", async (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A")]);

  const result = await start("board");

  assert.equal(dirs.story("US-5").blocked, false);
  assert.deepEqual(dirs.story("US-5").decisions, [{ question: "US-5: Which colour is the badge?", choice: "Use option A", by: "human" }]);
  assert.deepEqual(dirs.story("US-5").links, [questionLink("th-1")]);
  assert.equal(result.pickable, 1);
});

test("keeps a story blocked, with no decision, while one of its threads has no conclusion", async (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1"), questionLink("th-2")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A"), thread("th-2", "US-5: Which size is the badge?")]);

  const result = await start("board");

  assert.equal(dirs.story("US-5").blocked, true);
  assert.deepEqual(dirs.story("US-5").decisions, []);
  assert.equal(result.pickable, 0);
});

test("records only the new thread's conclusion when an earlier concluded thread already has its human decision", async (t) => {
  const earlier = { question: "US-5: Which colour is the badge?", choice: "Use option A", by: "human" };
  const dirs = scratch(t, [story("US-5", { blocked: true, decisions: [earlier], links: [questionLink("th-1"), questionLink("th-2")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A"), thread("th-2", "US-5: Which size is the badge?", "Use option B")]);

  await start("board");

  assert.deepEqual(dirs.story("US-5").decisions, [earlier, { question: "US-5: Which size is the badge?", choice: "Use option B", by: "human" }]);
  assert.equal(dirs.story("US-5").blocked, false);
});

test("a story unblocked at start is not clarity-checked again and stays pickable", async (t) => {
  const dirs = scratch(t, [story("US-5", { blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-5: Which colour is the badge?", "Use option A")]);
  const clarity = clarityAgent({ "US-5": [blockingGap] });

  const result = await start("board", clarity.agent);

  assert.deepEqual(clarity.prompts, []);
  assert.equal(dirs.story("US-5").blocked, false);
  assert.equal(result.pickable, 1);
});

// ---- implement in worktrees -------------------------------------------------

function deliver(args, agent) {
  return workflow(args, agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), () => {});
}

// An agent that passes every clarity check and hands each implementer call, with
// its prompt, to the script given for the story key in its label. Reviewer calls
// (label `review:<key>`) go to the script in reviewByKey, else pass. Check calls
// (label `verify:<key>`) go to the script in verifyByKey, else report no criteria.
// Test author calls (label `tests:<key>`) go to the script in testsByKey, else add
// nothing; the test reviewer accepts, and the final check reports healthy.
function implAgent(byKey, reviewByKey = {}, verifyByKey = {}, testsByKey = {}) {
  const prompts = {};
  const reviews = {};
  const verifies = {};
  const agent = async (prompt, opts) => {
    if (opts.label.startsWith("tests:")) {
      const key = opts.label.slice(6).split(" ")[0];
      return testsByKey[key] ? testsByKey[key]() : noTests;
    }
    if (opts.label.startsWith("test-review:")) return { pass: true, duplicates: [], issues: [], suiteGreen: true, productionUntouched: true, leftoverFiles: [] };
    if (opts.label === "final check") return { healthy: true, journeys: ["Board to badge: worked"], report: "fast passed" };
    if (opts.label.startsWith("verify:")) {
      const key = opts.label.slice(7).split(" ")[0];
      verifies[key] = (verifies[key] || 0) + 1;
      return verifyByKey[key] ? verifyByKey[key](verifies[key]) : { criteria: [] };
    }
    if (opts.label.startsWith("review:")) {
      const key = opts.label.slice(7).split(" ")[0];
      (reviews[key] ||= []).push(prompt);
      return reviewByKey[key] ? reviewByKey[key](reviews[key].length) : { verdict: "pass", findings: [] };
    }
    const key = opts.label.startsWith("impl:") && opts.label.slice(5).split(" ")[0];
    if (!key) return { gaps: [] };
    (prompts[key] ||= []).push(prompt);
    return byKey[key](prompts[key].length);
  };
  return { agent, prompts, reviews, verifies };
}

const noTests = { status: "done", commit: "", oracle: [], existingCoverage: [], added: [], reused: [], changedOrRemoved: [], notAdmitted: [] };
const threadOf = (dirs, key) => dirs.state().threads.find((th) => th.id === dirs.story(key).links[0].thread_id);
const deferred = () => {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
};

test("a story sharing a planned path with an implementing story is not picked beside it; the next free story is", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { files_planned: ["src/board.rs"] }), story("US-2", { files_planned: ["src/board.rs"] }), story("US-3", { files_planned: ["src/inbox.rs"] })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const thirdStarted = deferred();
  const seen = {};
  const { agent } = implAgent({
    "US-1": async () => {
      await thirdStarted.promise;
      seen.us2 = dirs.story("US-2").status;
      return { status: "ready", summary: "done" };
    },
    "US-2": async () => ({ status: "ready", summary: "done" }),
    "US-3": async () => {
      thirdStarted.resolve();
      return { status: "ready", summary: "done" };
    },
  });

  await deliver({ feature: "board", implementers: 2 }, agent);

  assert.equal(seen.us2, "backlog");
});

test("an implementer returning needs_decision blocks its story with a thread 'US-<n>: <question>' holding the options and recommendation, worktree kept", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const { agent } = implAgent({
    "US-1": async () => ({ status: "needs_decision", summary: "stopped", question: "Which colour is the badge?", options: ["Paint it red", "Paint it blue"], recommendation: "Paint it red" }),
  });

  await deliver({ feature: "board" }, agent);

  const th = threadOf(dirs, "US-1");
  assert.equal(th.title, "US-1: Which colour is the badge?");
  assert.ok(th.posts[0].body.includes("Paint it red") && th.posts[0].body.includes("Paint it blue"));
  assert.ok(th.posts[0].body.includes("Recommendation: Paint it red"));
  assert.deepEqual(dirs.story("US-1").links, [{ thread_id: th.id, label: "question" }]);
  assert.equal(dirs.story("US-1").blocked, true);
  assert.equal(existsSync(join(dirs.root, "app-stories", "US-1")), true);
});

test("records the implementer's decisions as agent decisions", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const { agent } = implAgent({
    "US-1": async () => ({ status: "ready", summary: "done", decisions: [{ question: "Where does the board query live?", choice: "In stories.rs" }] }),
  });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(dirs.story("US-1").decisions, [{ question: "Where does the board query live?", choice: "In stories.rs", by: "agent" }]);
});

// ---- the implement loop: claim, then implement ----------------------------------

// The run's id, from the lock it holds while it runs.
const runOf = (dirs) => JSON.parse(readFileSync(dirs.lock, "utf8")).run;

test("with implementers 2 and three pickable stories, each is implementing for this run before its implementer starts, two at once, and the third is claimed only when one finishes", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const thirdStarted = deferred();
  const atStart = {};
  const board = (dirs) => Object.fromEntries(dirs.state().stories.map((s) => [s.key, { status: s.status, run: s.holder && s.holder.run }]));
  const { agent } = implAgent({
    "US-1": async () => {
      atStart["US-1"] = { run: runOf(dirs), board: board(dirs) };
      await thirdStarted.promise;
      return { status: "ready", summary: "done" };
    },
    "US-2": async () => {
      atStart["US-2"] = { run: runOf(dirs), board: board(dirs) };
      return { status: "ready", summary: "done" };
    },
    "US-3": async () => {
      atStart["US-3"] = { run: runOf(dirs), board: board(dirs) };
      thirdStarted.resolve();
      return { status: "ready", summary: "done" };
    },
  });

  await deliver({ feature: "board", implementers: 2 }, agent);

  const run = atStart["US-1"].run;
  assert.deepEqual([atStart["US-1"].board["US-1"], atStart["US-1"].board["US-3"]], [{ status: "implementing", run }, { status: "backlog", run: null }]);
  assert.deepEqual(atStart["US-2"].board, { "US-1": { status: "implementing", run }, "US-2": { status: "implementing", run }, "US-3": { status: "backlog", run: null } });
  assert.deepEqual(atStart["US-3"].board, { "US-1": { status: "implementing", run }, "US-2": { status: "implemented", run: null }, "US-3": { status: "implementing", run } });
});

test("a finished implementer leaves its story implemented with no holder, its worktree in place and files_changed from its commits", async (t) => {
  const dirs = scratch(t, [story("US-1", { files_planned: ["src/other.rs"] })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = join(dirs.root, "app-stories", "US-1");
  const { agent } = implAgent({
    "US-1": async () => {
      writeFileSync(join(wt, "board.rs"), "board\n");
      git(wt, "add", "board.rs");
      git(wt, "commit", "-q", "-m", "feat: Board");
      return { status: "ready", summary: "done" };
    },
  });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(claimsOf(dirs, "US-1").map((c) => c.stage), ["implementing", "review"]);
  assert.equal(dirs.story("US-1").holder, null);
  assert.deepEqual(dirs.story("US-1").files_changed, ["board.rs"]);
  assert.equal(git(wt, "log", "-1", "--format=%s").trim(), "feat: Board");
});

test("a backlog story whose worktree exists at claim time keeps that worktree's commit and gets an implementer told to continue from it", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = worktree(dirs, "US-1");
  writeFileSync(join(wt, "board.rs"), "earlier attempt\n");
  git(wt, "add", "board.rs");
  git(wt, "commit", "-q", "-m", "feat: Earlier attempt");
  const seen = {};
  const { agent, prompts } = implAgent({
    "US-1": async () => {
      seen.head = git(wt, "log", "-1", "--format=%s").trim();
      return { status: "ready", summary: "done" };
    },
  });

  await deliver({ feature: "board" }, agent);

  assert.equal(seen.head, "feat: Earlier attempt");
  assert.match(prompts["US-1"][0], /Continue from it/);
});

test("with storyLimit 1, a story blocked by its implementer is claimed again once its thread concludes, and no other pickable story is claimed", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const statePath = join(dirs.root, "omni-state.json");
  // The human answers the implementer's question as soon as the run next looks for answers.
  const phase = (title) => {
    if (title !== "Unblock") return;
    const state = dirs.state();
    const us1 = state.stories.find((s) => s.key === "US-1");
    for (const th of state.threads.filter((th) => us1.links.some((l) => l.thread_id === th.id))) th.conclusion = "Paint it red";
    writeFileSync(statePath, JSON.stringify(state));
  };
  const { agent } = implAgent({
    "US-1": async (n) => (n === 1 ? { status: "needs_decision", summary: "stopped", question: "Which colour is the badge?", options: ["Paint it red", "Paint it blue"], recommendation: "Paint it red" } : { status: "ready", summary: "done" }),
    "US-2": async () => ({ status: "ready", summary: "done" }),
  });

  await workflow({ feature: "board", storyLimit: 1 }, agent, phase, (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(claimsOf(dirs, "US-1").map((c) => c.stage), ["implementing", "implementing", "review"]);
  assert.deepEqual(claimsOf(dirs, "US-2"), []);
  assert.equal(dirs.story("US-1").status, "reviewed");
  assert.equal(dirs.story("US-2").status, "backlog");
});

test("a pickable story whose clarity check returned nothing is not claimed; the next pickable story that passed is", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const impl = implAgent({
    "US-1": async () => ({ status: "ready", summary: "done" }),
    "US-2": async () => ({ status: "ready", summary: "done" }),
  });
  const agent = async (prompt, opts) => (opts.label === "clarity:US-1" ? null : impl.agent(prompt, opts));

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(claimsOf(dirs, "US-1"), []);
  assert.equal(dirs.story("US-1").status, "backlog");
  assert.deepEqual(claimsOf(dirs, "US-2").map((c) => c.stage), ["implementing", "review"]);
});

// ---- the review loop: claim, then review ----------------------------------------

// An implemented story with its worktree holding one commit of its attempt.
function implemented(dirs, key) {
  const wt = worktree(dirs, key);
  writeFileSync(join(wt, `${key}.rs`), "attempt\n");
  git(wt, "add", `${key}.rs`);
  git(wt, "commit", "-q", "-m", `feat: ${key} attempt`);
  return wt;
}
const hasBranch = (dirs, key) => git(dirs.repo, "branch", "--list", `story/${key}`).trim() !== "";

test("with reviewers 2, each implemented story is review for this run before its reviewer starts, both review at once, and a pass leaves it reviewed with no holder and its worktree", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented" }), story("US-2", { status: "implemented" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt1 = implemented(dirs, "US-1");
  const wt2 = implemented(dirs, "US-2");
  const secondStarted = deferred();
  const atStart = {};
  const board = (dirs) => Object.fromEntries(dirs.state().stories.map((s) => [s.key, { status: s.status, run: s.holder && s.holder.run }]));
  const { agent } = implAgent({}, {
    "US-1": async () => {
      atStart["US-1"] = { run: runOf(dirs), board: board(dirs) };
      // Waits for the second reviewer; with only one reviewer at a time it never comes.
      await Promise.race([secondStarted.promise, new Promise((r) => setTimeout(r, 3000))]);
      return { verdict: "pass", findings: [] };
    },
    "US-2": async () => {
      atStart["US-2"] = { run: runOf(dirs), board: board(dirs) };
      secondStarted.resolve();
      return { verdict: "pass", findings: [] };
    },
  });

  await deliver({ feature: "board", reviewers: 2 }, agent);

  const run = atStart["US-1"].run;
  assert.deepEqual(atStart["US-1"].board["US-1"], { status: "review", run });
  assert.deepEqual(atStart["US-2"].board, { "US-1": { status: "review", run }, "US-2": { status: "review", run } });
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-1").holder, dirs.story("US-2").status, dirs.story("US-2").holder], ["reviewed", null, "reviewed", null]);
  const movesOf = (key) => dirs.state().calls.filter((c) => c.argv[1] === "move" && c.argv[2] === key).map((c) => c.argv[c.argv.indexOf("--to") + 1]);
  assert.deepEqual([movesOf("US-1"), movesOf("US-2")], [["reviewed"], ["reviewed"]]);
  assert.equal(git(wt1, "log", "-1", "--format=%s").trim(), "feat: US-1 attempt");
  assert.equal(git(wt2, "log", "-1", "--format=%s").trim(), "feat: US-2 attempt");
});

test("the review loop claims implemented stories by priority, then by number", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented", priority: 3 }), story("US-10", { status: "implemented", priority: 1 }), story("US-2", { status: "implemented", priority: 1 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  for (const key of ["US-1", "US-10", "US-2"]) implemented(dirs, key);

  await deliver({ feature: "board", reviewers: 1 }, implAgent({}).agent);

  const order = dirs.state().calls.filter((c) => c.argv[1] === "claim").map((c) => c.argv[2]);
  assert.deepEqual(order, ["US-2", "US-10", "US-1"]);
});

test("a failing review sends its story to the backlog at priority 0 with [attempt 1] findings, no holder, worktree or branch; a fresh implementer gets the findings before it is reviewed again", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented", priority: 2 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = implemented(dirs, "US-1");
  const seen = {};
  const phase = (title) => {
    if (title !== "Unblock" || seen.sentBack || dirs.story("US-1").status !== "backlog") return;
    const s = dirs.story("US-1");
    seen.sentBack = { priority: s.priority, holder: s.holder, blocked: s.blocked, findings: s.findings.map((f) => f.text), worktree: existsSync(wt), branch: hasBranch(dirs, "US-1") };
  };
  const { agent, prompts } = implAgent(
    { "US-1": async () => {
      seen.head = git(wt, "log", "-1", "--format=%s").trim();
      return { status: "ready", summary: "done" };
    } },
    { "US-1": async (n) => (n === 1 ? { verdict: "fail", findings: [{ text: "The badge is blue, the criterion says red", files: ["src/badge.rs"], criterion: "The badge is red" }] } : { verdict: "pass", findings: [] }) },
  );

  await workflow({ feature: "board" }, agent, phase, (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(seen.sentBack, { priority: 0, holder: null, blocked: false, findings: ["[attempt 1] The badge is blue, the criterion says red"], worktree: false, branch: false });
  assert.deepEqual(claimsOf(dirs, "US-1").map((c) => c.stage), ["review", "implementing", "review"]);
  assert.equal(seen.head, "init");
  assert.match(prompts["US-1"][0], /\[attempt 1\] The badge is blue, the criterion says red/);
});

test("a third failed review leaves the story blocked in the backlog at priority 0, linked to 'US-1: still failing after 3 attempts', with no worktree or branch; once the thread concludes a fresh implementer starts", { timeout: 20000 }, async (t) => {
  const earlier = [{ stage: "review", text: "[attempt 1] The badge is missing", files: [], resolved: false }, { stage: "review", text: "[attempt 2] The badge is green", files: [], resolved: false }];
  const dirs = scratch(t, [story("US-1", { status: "implemented", priority: 0, findings: earlier })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const statePath = join(dirs.root, "omni-state.json");
  const wt = implemented(dirs, "US-1");
  const seen = {};
  // Records the story as the send-back left it, then the human answers its thread.
  const phase = (title) => {
    if (title !== "Unblock" || seen.sentBack) return;
    const state = dirs.state();
    const s = state.stories.find((x) => x.key === "US-1");
    if (!s.blocked) return;
    const th = state.threads.find((x) => x.id === s.links[0].thread_id);
    seen.sentBack = { status: s.status, priority: s.priority, holder: s.holder, links: s.links, title: th.title, findings: s.findings.map((f) => f.text), worktree: existsSync(wt), branch: hasBranch(dirs, "US-1") };
    th.conclusion = "Paint it red";
    writeFileSync(statePath, JSON.stringify(state));
  };
  const { agent, prompts } = implAgent(
    { "US-1": async () => {
      seen.head = git(wt, "log", "-1", "--format=%s").trim();
      return { status: "ready", summary: "done" };
    } },
    { "US-1": async (n) => (n === 1 ? { verdict: "fail", findings: [{ text: "The badge is blue", files: [], criterion: "The badge is red" }] } : { verdict: "pass", findings: [] }) },
  );

  await workflow({ feature: "board" }, agent, phase, (fns) => Promise.all(fns.map((f) => f())), () => {});

  const threadId = seen.sentBack.links[0].thread_id;
  assert.deepEqual(seen.sentBack, {
    status: "backlog",
    priority: 0,
    holder: null,
    links: [{ thread_id: threadId, label: "question" }],
    title: "US-1: still failing after 3 attempts",
    findings: ["[attempt 1] The badge is missing", "[attempt 2] The badge is green", "[attempt 3] The badge is blue"],
    worktree: false,
    branch: false,
  });
  assert.equal(prompts["US-1"].length, 1);
  assert.equal(seen.head, "init");
});
