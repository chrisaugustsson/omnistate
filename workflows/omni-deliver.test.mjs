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
    delete process.env.FAKE_OMNI_DOWN;
    delete process.env.FAKE_OMNI_HANG;
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

// The claims and moves the run sent for the story, in order: "claim <stage>" or "move <status>".
const timeline = (dirs, key) => dirs.state().calls.filter((c) => (c.argv[1] === "claim" || c.argv[1] === "move") && c.argv[2] === key).map((c) => (c.argv[1] === "claim" ? `claim ${c.argv[c.argv.indexOf("--stage") + 1]}` : `move ${c.argv[c.argv.indexOf("--to") + 1]}`));

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

test("working stories nobody holds are released to their waiting state like a dead run's, the implementing one's worktree kept, and the report lists them", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review" }), story("US-2", { status: "testing" }), story("US-3", { status: "implementing" }), story("US-4", { status: "implemented" })]);
  const wt = worktree(dirs, "US-3");
  writeFileSync(join(wt, "work"), "work\n");
  git(wt, "add", "work");
  git(wt, "commit", "-q", "-m", "work");

  const result = await start("board");

  assert.deepEqual(["US-1", "US-2", "US-3", "US-4"].map((k) => [dirs.story(k).status, dirs.story(k).holder]), [["implemented", null], ["reviewed", null], ["backlog", null], ["implemented", null]]);
  assert.equal(readFileSync(join(wt, "work"), "utf8"), "work\n");
  assert.deepEqual(releasedSection(result.report).sort(), ["- US-1: implemented", "- US-2: reviewed", "- US-3: backlog"]);
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

const noTests = { status: "done", commit: "", check: "", oracle: [], existingCoverage: [], added: [], reused: [], changedOrRemoved: [], notAdmitted: [] };
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

test("the implementer is told to run the stack tests only through the stack command's compose project from the main worktree's folder, never compose up or run from its worktree, and to say so in decisions when it cannot", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "compose-tool run --rm test" });
  const { agent, prompts } = implAgent({ "US-1": async () => ({ status: "ready", summary: "done" }) });

  await deliver({ feature: "board" }, agent);

  const rule = prompts["US-1"][0].split("\n").find((l) => l.startsWith("- The stack tests"));
  assert.ok(rule, "the rules name the stack tests");
  assert.match(rule, /through the configured stack command's compose project, `compose-tool run --rm test`, from the main worktree's folder /);
  assert.ok(rule.includes(`from the main worktree's folder ${dirs.repo}:`), "the rule names the main worktree's folder");
  assert.match(rule, /never `docker compose up` or `docker compose run` from your worktree/);
  assert.match(rule, /say so in decisions/);
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

  assert.deepEqual(claimsOf(dirs, "US-1").map((c) => c.stage), ["implementing", "review", "testing"]);
  assert.equal(dirs.story("US-1").holder, null);
  assert.deepEqual(dirs.story("US-1").files_changed, ["board.rs"]);
  // The worktree stayed for review and the lane, which merged its commit.
  assert.equal(git(dirs.repo, "log", "-1", "--format=%s", "main").trim(), "feat: Board");
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

  assert.deepEqual(claimsOf(dirs, "US-1").map((c) => c.stage), ["implementing", "implementing", "review", "testing"]);
  assert.deepEqual(claimsOf(dirs, "US-2"), []);
  assert.equal(dirs.story("US-1").status, "done");
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
  assert.deepEqual(claimsOf(dirs, "US-2").map((c) => c.stage), ["implementing", "review", "testing"]);
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
  implemented(dirs, "US-1");
  implemented(dirs, "US-2");
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
  assert.deepEqual([dirs.story("US-1").holder, dirs.story("US-2").holder], [null, null]);
  // A pass moves to reviewed; the lane then claims it (the fake allows that only
  // from reviewed) and merges the attempt kept in its worktree.
  const movesOf = (key) => dirs.state().calls.filter((c) => c.argv[1] === "move" && c.argv[2] === key).map((c) => c.argv[c.argv.indexOf("--to") + 1]);
  assert.deepEqual([movesOf("US-1"), movesOf("US-2")], [["reviewed", "done"], ["reviewed", "done"]]);
  assert.deepEqual([claimsOf(dirs, "US-1").map((c) => c.stage), claimsOf(dirs, "US-2").map((c) => c.stage)], [["review", "testing"], ["review", "testing"]]);
  assert.deepEqual(git(dirs.repo, "log", "--format=%s", "main").trim().split("\n").sort(), ["feat: US-1 attempt", "feat: US-2 attempt", "init"]);
});

test("the review loop claims implemented stories by priority, then by number", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented", priority: 3 }), story("US-10", { status: "implemented", priority: 1 }), story("US-2", { status: "implemented", priority: 1 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  for (const key of ["US-1", "US-10", "US-2"]) implemented(dirs, key);

  await deliver({ feature: "board", reviewers: 1 }, implAgent({}).agent);

  const order = dirs.state().calls.filter((c) => c.argv[1] === "claim" && c.argv.includes("review")).map((c) => c.argv[2]);
  assert.deepEqual(order, ["US-2", "US-10", "US-1"]);
});

test("a failing review sends its story to the backlog at priority 0 with [attempt 1] findings and no holder, its worktree and branch kept; a fresh implementer continues from them with the findings before it is reviewed again", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implemented", priority: 2 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = implemented(dirs, "US-1");
  const seen = {};
  // The story is claimed again right after the send-back, so the fresh implementer sees what it left.
  const { agent, prompts } = implAgent(
    { "US-1": async () => {
      const s = dirs.story("US-1");
      seen.atImplementer = { priority: s.priority, blocked: s.blocked, findings: s.findings.map((f) => f.text), head: git(wt, "log", "-1", "--format=%s").trim() };
      return { status: "ready", summary: "done" };
    } },
    { "US-1": async (n) => (n === 1 ? { verdict: "fail", findings: [{ text: "The badge is blue, the criterion says red", files: ["src/badge.rs"], criterion: "The badge is red" }] } : { verdict: "pass", findings: [] }) },
  );

  await deliver({ feature: "board" }, agent);

  // The attempt's worktree and branch stay, with its commit.
  assert.deepEqual(seen.atImplementer, { priority: 0, blocked: false, findings: ["[attempt 1] The badge is blue, the criterion says red"], head: "feat: US-1 attempt" });
  assert.deepEqual(timeline(dirs, "US-1"), ["claim review", "move backlog", "claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move done"]);
  assert.match(prompts["US-1"][0], /Continue from it/);
  assert.match(prompts["US-1"][0], /\[attempt 1\] The badge is blue, the criterion says red/);
});

test("a third failed review leaves the story blocked in the backlog at priority 0, linked to 'US-1: still failing after 3 attempts', its worktree and branch kept; once the thread concludes a fresh implementer continues from them", { timeout: 20000 }, async (t) => {
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
    worktree: true,
    branch: true,
  });
  assert.equal(prompts["US-1"].length, 1);
  assert.equal(seen.head, "feat: US-1 attempt");
});

// ---- the test lane: merge and check one story at a time -------------------------

// A reviewed story with its worktree holding one commit of its attempt.
const reviewed = implemented;
const subjects = (dirs) => git(dirs.repo, "log", "--format=%s", "main").trim().split("\n");
// A main-worktree commit to the file given, made after the stories branched off.
function mainEdits(dirs, file, content) {
  writeFileSync(join(dirs.repo, file), content);
  git(dirs.repo, "add", file);
  git(dirs.repo, "commit", "-q", "-m", `chore: main edits ${file}`);
}
// A check command that logs its step, the lane file (null once the lane removed
// it) and the main worktree's head commit as one JSON line, then exits with the
// code given.
const step = (log, name, code = 0, output = "") => `printf '{"step":"${name}","lane":%s,"head":"%s"}\\n' "$(test -e .omni/lane.json && tr -d '\\n' < .omni/lane.json || echo null)" "$(git log -1 --format=%s)" >> ${log}; echo '${output}'; exit ${code}`;
const stepsOf = (log) => (existsSync(log) ? readFileSync(log, "utf8").trim().split("\n").map((l) => JSON.parse(l)) : []);
const sha = (dirs, rev) => git(dirs.repo, "rev-parse", rev).trim();
// The story as the run left it right after a send-back, seen on the next tick.
function onSendBack(dirs, key, seen) {
  return (title) => {
    if (title !== "Unblock" || seen.sentBack || dirs.story(key).status !== "backlog") return;
    const s = dirs.story(key);
    seen.sentBack = { status: s.status, priority: s.priority, holder: s.holder, findings: s.findings.map((f) => ({ stage: f.stage, text: f.text })), main: subjects(dirs), clean: git(dirs.repo, "status", "--porcelain"), lane: existsSync(laneFile(dirs)) };
  };
}
const stopped = async () => ({ status: "blocked", summary: "stopped" });

test("two reviewed stories pass the lane one at a time: each rebased and fast-forwarded, then fast, stack_up, stack and the verifier in order, with the lane file naming it and the commit before its merge; the second is claimed only once the first is done", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" }), story("US-2", { status: "reviewed" })]);
  const log = join(dirs.root, "steps.log");
  dirs.checks({ fast: step(log, "fast"), stack_up: step(log, "stack_up"), stack: step(log, "stack") });
  reviewed(dirs, "US-1");
  reviewed(dirs, "US-2");
  mainEdits(dirs, "main.rs", "main\n");
  const seen = {};
  const verifier = (key) => () => {
    writeFileSync(log, readFileSync(log, "utf8") + JSON.stringify({ step: "verify", lane: JSON.parse(readFileSync(laneFile(dirs), "utf8")), head: subjects(dirs)[0] }) + "\n");
    seen[key] = { "US-1": dirs.story("US-1").status, "US-2": dirs.story("US-2").status, claimsOfUS2: claimsOf(dirs, "US-2").length };
    return { criteria: [] };
  };

  await deliver({ feature: "board" }, implAgent({}, {}, { "US-1": verifier("US-1"), "US-2": verifier("US-2") }).agent);

  assert.deepEqual(subjects(dirs), ["feat: US-2 attempt", "feat: US-1 attempt", "chore: main edits main.rs", "init"]);
  assert.equal(git(dirs.repo, "rev-list", "--merges", "main").trim(), "");
  const lane1 = { key: "US-1", base: sha(dirs, "main~2") };
  const lane2 = { key: "US-2", base: sha(dirs, "main~1") };
  assert.deepEqual(stepsOf(log), [
    { step: "fast", lane: lane1, head: "feat: US-1 attempt" },
    { step: "stack_up", lane: lane1, head: "feat: US-1 attempt" },
    { step: "stack", lane: lane1, head: "feat: US-1 attempt" },
    { step: "verify", lane: lane1, head: "feat: US-1 attempt" },
    { step: "fast", lane: lane2, head: "feat: US-2 attempt" },
    { step: "stack_up", lane: lane2, head: "feat: US-2 attempt" },
    { step: "stack", lane: lane2, head: "feat: US-2 attempt" },
    { step: "verify", lane: lane2, head: "feat: US-2 attempt" },
  ]);
  assert.deepEqual(seen["US-1"], { "US-1": "testing", "US-2": "reviewed", claimsOfUS2: 0 });
  assert.deepEqual(seen["US-2"], { "US-1": "done", "US-2": "testing", claimsOfUS2: 1 });
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-2").status], ["done", "done"]);
  assert.equal(existsSync(laneFile(dirs)), false);
});

test("the lane logs one line per step with the story key: claimed, merged, fast, stack, criteria, done; a failing check logs the send-back and its reason", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" }), story("US-2", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: `test "$(git log -1 --format=%s)" != "feat: US-2 attempt"` });
  reviewed(dirs, "US-1");
  reviewed(dirs, "US-2");
  const base = sha(dirs, "main").slice(0, 7);
  const lines = [];
  const { agent } = implAgent({ "US-2": stopped });

  await workflow({ feature: "board" }, agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.deepEqual(lines.filter((l) => l.startsWith("lane:US-1:")), [`lane:US-1: claimed (main at ${base})`, "lane:US-1: merged onto main", "lane:US-1: fast passed", "lane:US-1: stack passed", "lane:US-1: criteria passed", "lane:US-1: done"]);
  const base2 = sha(dirs, "main").slice(0, 7);
  assert.deepEqual(lines.filter((l) => l.startsWith("lane:US-2:")), [`lane:US-2: claimed (main at ${base2})`, "lane:US-2: merged onto main", "lane:US-2: fast passed", "lane:US-2: sent back: the stack command failed", `lane:US-2: rebuilding the stack from main at ${base2}`, `lane:US-2: stack rebuilt from main at ${base2}`]);
  assert.equal(dirs.story("US-1").status, "done");
  assert.deepEqual([dirs.story("US-2").status, dirs.story("US-2").priority], ["backlog", 0]);
});

// A stack command that takes the fake omni down with the message given until two
// polls have been refused (the pause must be logged once, not per poll), then
// brings it back.
function stackTakingOmniDown(dirs, message) {
  const down = join(dirs.root, "down");
  process.env.FAKE_OMNI_DOWN = down;
  return `printf '%s\\n' "${message}" > ${down}; i=0; until [ "$(grep -c '"refused": true' ${join(dirs.root, "omni-state.json")})" -ge 2 ] || [ $i -ge 60 ]; do sleep 0.25; i=$((i+1)); done; rm -f ${down}`;
}
const refusedPolls = (dirs) => dirs.state().calls.filter((c) => c.refused).length;

test("polls refused while the lane's stack command restarts the server log one 'lane: stack restarting, polls paused' line and no Command failed line, and the lane goes on to done", { timeout: 30000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: stackTakingOmniDown(dirs, "error: cannot reach the server: Connection refused") });
  reviewed(dirs, "US-1");
  const lines = [];

  await workflow({ feature: "board" }, implAgent({}).agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.ok(refusedPolls(dirs) >= 2, `two or more polls were refused while the stack ran, not ${refusedPolls(dirs)}`);
  assert.deepEqual(lines.filter((l) => l === "lane: stack restarting, polls paused"), ["lane: stack restarting, polls paused"]);
  assert.deepEqual(lines.filter((l) => /Command failed/.test(l)), []);
  assert.equal(dirs.story("US-1").status, "done");
});

test("a poll failing for another reason while the lane's stack command runs still logs Command failed with the message, and no paused line", { timeout: 30000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: stackTakingOmniDown(dirs, "error: the board is on fire") });
  reviewed(dirs, "US-1");
  const lines = [];

  await workflow({ feature: "board" }, implAgent({}).agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.ok(refusedPolls(dirs) >= 2, `two or more polls were refused while the stack ran, not ${refusedPolls(dirs)}`);
  assert.ok(lines.some((l) => /^(unblock|pull): Command failed: omni story list .*the board is on fire/s.test(l)), `a failed poll is logged: ${JSON.stringify(lines)}`);
  assert.deepEqual(lines.filter((l) => l === "lane: stack restarting, polls paused"), []);
});

test("the lane claims reviewed stories by priority, then by number", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", priority: 3 }), story("US-10", { status: "reviewed", priority: 1 }), story("US-2", { status: "reviewed", priority: 1 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  for (const key of ["US-1", "US-10", "US-2"]) reviewed(dirs, key);

  await deliver({ feature: "board" }, implAgent({}).agent);

  const order = dirs.state().calls.filter((c) => c.argv[1] === "claim" && c.argv.includes("testing")).map((c) => c.argv[2]);
  assert.deepEqual(order, ["US-2", "US-10", "US-1"]);
});

test("a fast command failing in the lane resets the integration branch to the lane base, removes the lane file, runs nothing after it, and sends the story to the backlog at priority 0 with an [attempt 1] testing finding naming the command and its output", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", priority: 2, criteria: [{ text: "The badge has a colour" }] })]);
  const log = join(dirs.root, "steps.log");
  dirs.checks({ fast: step(log, "fast", 1, "fast broke on main"), stack_up: step(log, "stack_up"), stack: step(log, "stack") });
  reviewed(dirs, "US-1");
  mainEdits(dirs, "main.rs", "main\n");
  const seen = {};
  const { agent, verifies } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(stepsOf(log).map((s) => s.step), ["fast"]);
  assert.equal(verifies["US-1"], undefined);
  const { findings, ...rest } = seen.sentBack;
  assert.deepEqual(rest, { status: "backlog", priority: 0, holder: null, main: ["chore: main edits main.rs", "init"], clean: "", lane: false });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].stage, "testing");
  assert.match(findings[0].text, /^\[attempt 1\] The fast command .*failed/);
  assert.match(findings[0].text, /fast broke on main/);
});

test("a stack command failing in the lane resets the branch to the lane base, runs no verifier, sends the story back with an [attempt 1] finding naming the command and its output, and then runs stack_up again at the lane base", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", criteria: [{ text: "The badge has a colour" }] })]);
  const log = join(dirs.root, "steps.log");
  dirs.checks({ fast: step(log, "fast"), stack_up: step(log, "stack_up"), stack: step(log, "stack", 1, "the stack broke") });
  reviewed(dirs, "US-1");
  const seen = {};
  const { agent, verifies } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(stepsOf(log).map((s) => s.step), ["fast", "stack_up", "stack", "stack_up"]);
  assert.deepEqual(stepsOf(log)[3], { step: "stack_up", lane: null, head: "init" });
  assert.equal(verifies["US-1"], undefined);
  assert.deepEqual([seen.sentBack.main, seen.sentBack.lane, seen.sentBack.priority], [["init"], false, 0]);
  assert.equal(seen.sentBack.findings.length, 1);
  assert.match(seen.sentBack.findings[0].text, /^\[attempt 1\] The stack command .*failed/);
  assert.match(seen.sentBack.findings[0].text, /the stack broke/);
});

// A stack_up command that logs the main worktree's head commit and whether the
// lane file exists, so a run of it after the lane's reset can be told apart.
const stackUpStep = (log) => `echo "stack_up $(git log -1 --format=%s) $(test -e .omni/lane.json && echo lane || echo no-lane)" >> ${log}`;

test("a stack command failing after stack_up ran leaves the stack rebuilt: stack_up runs again from the reset tree, after the send-back and without a lane file, and is logged; a story that passes the stack command runs stack_up once", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" }), story("US-2", { status: "reviewed" })]);
  const log = join(dirs.root, "steps.log");
  dirs.checks({ fast: "true", stack_up: stackUpStep(log), stack: `echo "stack $(git log -1 --format=%s)" >> ${log}; test "$(git log -1 --format=%s)" != "feat: US-2 attempt"` });
  reviewed(dirs, "US-1");
  reviewed(dirs, "US-2");
  const lines = [];
  const { agent } = implAgent({ "US-2": stopped });

  await workflow({ feature: "board" }, agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.deepEqual(readFileSync(log, "utf8").trim().split("\n"), [
    "stack_up feat: US-1 attempt lane",
    "stack feat: US-1 attempt",
    "stack_up feat: US-2 attempt lane",
    "stack feat: US-2 attempt",
    "stack_up feat: US-1 attempt no-lane",
  ]);
  const base = sha(dirs, "main").slice(0, 7);
  assert.deepEqual(lines.filter((l) => l.startsWith("lane:US-2:")).slice(-3), ["lane:US-2: sent back: the stack command failed", `lane:US-2: rebuilding the stack from main at ${base}`, `lane:US-2: stack rebuilt from main at ${base}`]);
  assert.deepEqual(lines.filter((l) => l.startsWith("lane:US-1:") && /stack/.test(l)), ["lane:US-1: stack passed"]);
  assert.equal(dirs.story("US-1").status, "done");
  assert.equal(dirs.story("US-2").status, "backlog");
});

test("a fast command failing in the lane runs stack_up neither before nor after the send-back: the stack never held the story's build, so the renderer the web image copies from the fast command is never stale", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const log = join(dirs.root, "steps.log");
  dirs.checks({ fast: "false", stack_up: stackUpStep(log), stack: "true" });
  reviewed(dirs, "US-1");
  const lines = [];
  const { agent } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.equal(existsSync(log), false);
  assert.deepEqual(lines.filter((l) => /rebuil/.test(l)), []);
});

test("a stack command reporting a file missing gets a finding with the host's view before the reset: HEAD at the merged commit, the dirty tree, the file present on the host with its size, and the compose containers", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", criteria: [{ text: "The badge has a colour" }] })]);
  const head = join(dirs.root, "head");
  // The test container's view of the main worktree: a path under its mount that the host holds as README (4 bytes).
  dirs.checks({ fast: "true", stack_up: "true", stack: `git rev-parse HEAD > ${head}; touch scratch.tmp; echo "error: couldn't read /workspace/README: No such file or directory (os error 2)"; exit 1` });
  reviewed(dirs, "US-1");
  const seen = {};
  const { agent } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  const text = seen.sentBack.findings[0].text;
  const [output, evidence] = text.split("Host at that moment:\n");
  assert.match(output, /^\[attempt 1\] The stack command .*failed on the main worktree\. Output:\n```\n.*No such file or directory.*\n```\n$/s);
  const merged = readFileSync(head, "utf8").trim();
  assert.notEqual(merged, sha(dirs, "main"), "the branch was reset after the evidence was taken");
  assert.match(evidence, new RegExp(`^- HEAD: ${merged}\n`));
  assert.match(evidence, /- git status --short: \n\?\? scratch\.tmp\n/);
  assert.match(evidence, new RegExp(`- paths the output reports missing: \n  /workspace/README: present on the host at ${dirs.repo}/README \\(4 bytes\\)\n`));
  assert.match(evidence, /- docker compose ps: \S/);
});

test("a stack_up failing in the lane runs neither the stack command nor the verifier, sends the story back with the branch at the lane base, and runs stack_up again there", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", criteria: [{ text: "The badge has a colour" }] })]);
  const log = join(dirs.root, "steps.log");
  dirs.checks({ fast: step(log, "fast"), stack_up: step(log, "stack_up", 1, "the stack did not start"), stack: step(log, "stack") });
  reviewed(dirs, "US-1");
  const seen = {};
  const { agent, verifies } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(stepsOf(log).map((s) => s.step), ["fast", "stack_up", "stack_up"]);
  assert.deepEqual(stepsOf(log)[2], { step: "stack_up", lane: null, head: "init" });
  assert.equal(verifies["US-1"], undefined);
  assert.deepEqual([seen.sentBack.main, seen.sentBack.lane, seen.sentBack.findings.length], [["init"], false, 1]);
  assert.match(seen.sentBack.findings[0].text, /^\[attempt 1\] .*the stack did not start/s);
});

test("criteria failing in the lane reset the branch to the lane base and send the story back with one [attempt 1] testing finding per failed criterion, naming it and the evidence", { timeout: 20000 }, async (t) => {
  const criteria = [{ text: "The badge has a colour" }, { text: "The badge shows the count" }, { text: "The badge is red" }];
  const dirs = scratch(t, [story("US-1", { status: "reviewed", criteria })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  reviewed(dirs, "US-1");
  const seen = {};
  const verdict = { criteria: [{ index: 0, criterion: "The badge has a colour", ok: true, evidence: "ran it" }, { index: 1, criterion: "The badge shows the count", ok: false, evidence: "the count is missing" }, { index: 2, criterion: "The badge is red", ok: false, evidence: "it is blue" }] };
  const { agent } = implAgent({ "US-1": stopped }, {}, { "US-1": () => verdict });

  await workflow({ feature: "board" }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(seen.sentBack, {
    status: "backlog",
    priority: 0,
    holder: null,
    findings: [{ stage: "testing", text: "[attempt 1] The badge shows the count: the count is missing" }, { stage: "testing", text: "[attempt 1] The badge is red: it is blue" }],
    main: ["init"],
    clean: "",
    lane: false,
  });
});

test("a rebase conflict leaves the integration branch untouched and sends the story back with the rebase finding and no attempt counted; the next reviewed story is taken and done", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", priority: 2 }), story("US-2", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = worktree(dirs, "US-1");
  writeFileSync(join(wt, "README"), "app\nfrom US-1\n");
  git(wt, "commit", "-q", "-am", "feat: US-1 attempt");
  reviewed(dirs, "US-2");
  mainEdits(dirs, "README", "app\nfrom main\n");
  const seen = {};
  // The lane takes US-2 right after US-1's conflict: the board as US-1 left it, seen as the lane starts again.
  const phase = (title) => {
    if (title !== "Test lane" || !claimsOf(dirs, "US-2").length || seen.atNext) return;
    seen.atNext = { main: subjects(dirs), clean: git(dirs.repo, "status", "--porcelain"), lane: existsSync(laneFile(dirs)) };
  };
  const { agent } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, phase, (fns) => Promise.all(fns.map((f) => f())), () => {});

  assert.deepEqual(seen.atNext, { main: ["chore: main edits README", "init"], clean: "", lane: false });
  assert.deepEqual(timeline(dirs, "US-1").slice(0, 2), ["claim testing", "move backlog"]);
  const s = dirs.story("US-1");
  assert.equal(s.priority, 0);
  assert.equal(s.findings.length, 1);
  assert.equal(s.findings[0].stage, "testing");
  assert.match(s.findings[0].text, /^rebase conflict in README/);
  assert.deepEqual(claimsOf(dirs, "US-2").map((c) => c.stage), ["testing"]);
  assert.equal(dirs.story("US-2").status, "done");
  assert.deepEqual(subjects(dirs), ["feat: US-2 attempt", "chore: main edits README", "init"]);
});

test("a reviewed story whose branch is gone is sent back with a missing-branch finding at stage testing and no attempt counted, the integration branch untouched and no lane file; the next reviewed story is taken and done", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", priority: 2 }), story("US-2", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  reviewed(dirs, "US-2");
  const seen = {};
  const phase = (title) => {
    if (title !== "Test lane" || !claimsOf(dirs, "US-2").length || seen.atNext) return;
    seen.atNext = { main: subjects(dirs), clean: git(dirs.repo, "status", "--porcelain"), lane: existsSync(laneFile(dirs)) };
  };
  const lines = [];
  const base = sha(dirs, "main").slice(0, 7);
  const { agent } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board" }, agent, phase, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.deepEqual(seen.atNext, { main: ["init"], clean: "", lane: false });
  assert.deepEqual(timeline(dirs, "US-1").slice(0, 2), ["claim testing", "move backlog"]);
  const s = dirs.story("US-1");
  assert.equal(s.priority, 0);
  assert.deepEqual(s.findings.map((f) => [f.stage, f.text]), [["testing", "branch story/US-1 is missing: nothing to merge onto main; start again from main"]]);
  assert.deepEqual(lines.filter((l) => l.startsWith("lane:US-1:")), [`lane:US-1: claimed (main at ${base})`, "lane:US-1: sent back: branch story/US-1 is missing"]);
  assert.equal(dirs.story("US-2").status, "done");
  assert.deepEqual(subjects(dirs), ["feat: US-2 attempt", "init"]);
});

// ---- the test lane: regression tests, then done ---------------------------------


test("a found_defect from the test author resets the integration branch to the lane base, its test commit gone, and sends the story back with an [attempt 1] testing finding naming the test, the observed and the expected value", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed", priority: 2 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  reviewed(dirs, "US-1");
  mainEdits(dirs, "main.rs", "main\n");
  const seen = {};
  const author = () => {
    writeFileSync(join(dirs.repo, "badge.test.mjs"), "test\n");
    git(dirs.repo, "add", "badge.test.mjs");
    git(dirs.repo, "commit", "-q", "-m", "test: Badge colour");
    return { ...noTests, status: "found_defect", defect: { test: "a late order's badge is red", observed: "the badge is blue", expected: "the badge is red" } };
  };
  const { agent } = implAgent({ "US-1": stopped }, {}, {}, { "US-1": author });

  await workflow({ feature: "board" }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  const { findings, ...rest } = seen.sentBack;
  assert.deepEqual(rest, { status: "backlog", priority: 0, holder: null, main: ["chore: main edits main.rs", "init"], clean: "", lane: false });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].stage, "testing");
  assert.match(findings[0].text, /^\[attempt 1\] /);
  assert.match(findings[0].text, /a late order's badge is red/);
  assert.match(findings[0].text, /the badge is blue/);
  assert.match(findings[0].text, /the badge is red/);
});

test("the lane restores what the test author left uncommitted, keeps ignored files, runs the author's check command on its test commit, and the story is done with its tests on main", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = reviewed(dirs, "US-1");
  mainEdits(dirs, ".gitignore", ".omni/\ntarget/\n");
  mkdirSync(join(dirs.repo, "target"));
  writeFileSync(join(dirs.repo, "target", "build.out"), "build\n");
  const checks = join(dirs.root, "checks.log");
  // Commits a test file, and leaves a tracked file edited and a new test file uncommitted.
  const author = () => {
    writeFileSync(join(dirs.repo, "badge.test.mjs"), "test\n");
    git(dirs.repo, "add", "badge.test.mjs");
    git(dirs.repo, "commit", "-q", "-m", "test: Badge colour");
    writeFileSync(join(dirs.repo, "README"), "app\nedited\n");
    writeFileSync(join(dirs.repo, "wip.test.mjs"), "wip\n");
    return { ...noTests, commit: "head", check: `git log -1 --format=%s >> ${checks}; git status --porcelain >> ${checks}; test -e badge.test.mjs`, added: [{ test: "a late order's badge is red", file: "badge.test.mjs", risk: "the badge", overlapCheck: "none", sensitivity: "red on blue" }] };
  };

  const result = await deliver({ feature: "board" }, implAgent({}, {}, {}, { "US-1": author }).agent);

  assert.deepEqual(subjects(dirs), ["test: Badge colour", "feat: US-1 attempt", "chore: main edits .gitignore", "init"]);
  assert.equal(readFileSync(checks, "utf8"), "test: Badge colour\n", "the check ran at the test commit, on a clean tree");
  assert.equal(git(dirs.repo, "status", "--porcelain"), "");
  assert.doesNotMatch(readFileSync(join(dirs.repo, "README"), "utf8"), /edited/);
  assert.equal(readFileSync(join(dirs.repo, "target", "build.out"), "utf8"), "build\n");
  assert.equal(existsSync(laneFile(dirs)), false);
  assert.deepEqual([dirs.story("US-1").status, existsSync(wt), hasBranch(dirs, "US-1")], ["done", false, false]);
  assert.match(result.report, /US-1: 1 added \(a late order's badge is red\), 0 reused, 0 removed$/m);
});

test("a check command that fails drops the author's test commits, leaves the story done on its verified implementation, and the report names the dropped tests", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  reviewed(dirs, "US-1");
  const lines = [];
  const author = () => {
    writeFileSync(join(dirs.repo, "badge.test.mjs"), "test\n");
    git(dirs.repo, "add", "badge.test.mjs");
    git(dirs.repo, "commit", "-q", "-m", "test: Badge colour");
    return { ...noTests, commit: "head", check: "echo 'badge: expected red, got blue'; exit 1", added: [{ test: "a late order's badge is red", file: "badge.test.mjs", risk: "the badge", overlapCheck: "none", sensitivity: "red on blue" }] };
  };

  const result = await workflow({ feature: "board" }, implAgent({}, {}, {}, { "US-1": author }).agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.deepEqual(subjects(dirs), ["feat: US-1 attempt", "init"]);
  assert.equal(git(dirs.repo, "status", "--porcelain"), "");
  assert.equal(dirs.story("US-1").status, "done");
  assert.ok(lines.includes("lane:US-1: tests dropped: `echo 'badge: expected red, got blue'; exit 1` failed"), lines.filter((l) => l.startsWith("lane:")).join("\n"));
  assert.match(result.report, /US-1: 0 added, 0 reused, 0 removed; tests dropped: `echo 'badge: expected red, got blue'; exit 1` failed:$/m);
});

// ---- the end of the run: nothing waits and nothing is held ----------------------

test("a run whose only unblocked backlog story waits on an open story that is itself blocked ends with no loop agent started and nothing claimed", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { blocked: true, links: [questionLink("th-1")] }), story("US-2", { blocked_by: ["US-1"] })], [thread("th-1", "US-1: Which colour is the badge?")]);
  const labels = [];
  const agent = async (prompt, opts) => {
    labels.push(opts.label);
    return { gaps: [] };
  };

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(labels, ["clarity:US-2"]);
  assert.deepEqual(dirs.state().calls.filter((c) => c.argv[1] === "claim"), []);
  assert.deepEqual([dirs.story("US-1").status, dirs.story("US-2").status], ["backlog", "backlog"]);
});

test("a backlog, an implemented and a reviewed story are pulled at once by the implement loop, the review loop and the lane, and each runs to done", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2", { status: "implemented" }), story("US-3", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  implemented(dirs, "US-2");
  reviewed(dirs, "US-3");
  const laneChecked = deferred();
  const seen = {};
  const statuses = () => Object.fromEntries(dirs.state().stories.map((s) => [s.key, s.status]));
  // The implementer and the reviewer wait for the lane's verifier; if the loops ran one after another it never comes.
  const meanwhile = () => Promise.race([laneChecked.promise, new Promise((r) => setTimeout(r, 3000))]);
  const wt = join(dirs.root, "app-stories", "US-1");
  const { agent } = implAgent(
    { "US-1": async () => {
      await meanwhile();
      writeFileSync(join(wt, "US-1.rs"), "board\n");
      git(wt, "add", "US-1.rs");
      git(wt, "commit", "-q", "-m", "feat: US-1 attempt");
      return { status: "ready", summary: "done" };
    } },
    { "US-2": async () => {
      await meanwhile();
      return { verdict: "pass", findings: [] };
    } },
    { "US-3": () => {
      seen.atLaneCheck = statuses();
      laneChecked.resolve();
      return { criteria: [] };
    } },
  );

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(seen.atLaneCheck, { "US-1": "implementing", "US-2": "review", "US-3": "testing" });
  assert.deepEqual(timeline(dirs, "US-1"), ["claim implementing", "move implemented", "claim review", "move reviewed", "claim testing", "move done"]);
  assert.deepEqual(timeline(dirs, "US-2"), ["claim review", "move reviewed", "claim testing", "move done"]);
  assert.deepEqual(timeline(dirs, "US-3"), ["claim testing", "move done"]);
  assert.deepEqual(statuses(), { "US-1": "done", "US-2": "done", "US-3": "done" });
});

test("an implementer that starts while the lane holds a merge cuts its worktree from the lane base, not the merged head, records that base on its branch, and every story still merges", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  reviewed(dirs, "US-3");
  const init = git(dirs.repo, "rev-parse", "HEAD").trim();
  const laneMerged = deferred();
  const secondCut = deferred();
  const seen = {};
  const wt1 = join(dirs.root, "app-stories", "US-1");
  const wt2 = join(dirs.root, "app-stories", "US-2");
  const { agent } = implAgent(
    {
      "US-1": async () => {
        // The lane merges US-3 onto main meanwhile; US-2 is cut only after this returns.
        await laneMerged.promise;
        writeFileSync(join(wt1, "US-1.rs"), "board\n");
        git(wt1, "add", "US-1.rs");
        git(wt1, "commit", "-q", "-m", "feat: US-1 attempt");
        return { status: "ready", summary: "done" };
      },
      "US-2": async () => {
        seen.mainHead = git(dirs.repo, "log", "-1", "--format=%s", "main").trim();
        seen.base = git(wt2, "rev-parse", "HEAD").trim();
        seen.recorded = git(dirs.repo, "config", "--get", "branch.story/US-2.omnibase").trim();
        secondCut.resolve();
        writeFileSync(join(wt2, "US-2.rs"), "board\n");
        git(wt2, "add", "US-2.rs");
        git(wt2, "commit", "-q", "-m", "feat: US-2 attempt");
        return { status: "ready", summary: "done" };
      },
    },
    {},
    {
      "US-3": async () => {
        // US-3 is merged on main now; hold the lane here until US-2's worktree is cut.
        laneMerged.resolve();
        await Promise.race([secondCut.promise, new Promise((r) => setTimeout(r, 5000))]);
        return { criteria: [] };
      },
    },
  );

  await deliver({ feature: "board", implementers: 1 }, agent);

  assert.equal(seen.mainHead, "feat: US-3 attempt");
  assert.equal(seen.base, init);
  assert.equal(seen.recorded, init);
  assert.deepEqual(Object.fromEntries(dirs.state().stories.map((s) => [s.key, s.status])), { "US-1": "done", "US-2": "done", "US-3": "done" });
  assert.deepEqual(subjects(dirs).sort(), ["feat: US-1 attempt", "feat: US-2 attempt", "feat: US-3 attempt", "init"]);
});

test("the lane rebases a story from the base recorded on its branch, so a merge the integration branch has since reset away is not replayed", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const init = git(dirs.repo, "rev-parse", "HEAD").trim();
  mainEdits(dirs, "dropped.rs", "a story the lane merged and later reset away\n");
  const merged = git(dirs.repo, "rev-parse", "HEAD").trim();
  reviewed(dirs, "US-1");
  git(dirs.repo, "config", "branch.story/US-1.omnibase", merged);
  git(dirs.repo, "reset", "-q", "--hard", init);
  const { agent } = implAgent({});

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(subjects(dirs), ["feat: US-1 attempt", "init"]);
  assert.equal(dirs.story("US-1").status, "done");
});

test("without a recorded base the lane rebases from git's fork point, with the same result", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const init = git(dirs.repo, "rev-parse", "HEAD").trim();
  mainEdits(dirs, "dropped.rs", "a story the lane merged and later reset away\n");
  reviewed(dirs, "US-1");
  git(dirs.repo, "reset", "-q", "--hard", init);
  const { agent } = implAgent({});

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(subjects(dirs), ["feat: US-1 attempt", "init"]);
  assert.equal(dirs.story("US-1").status, "done");
});

test("a branch with no recorded base whose tip was itself on the integration branch before is rebased from the merge base, so its own commit is kept", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const init = git(dirs.repo, "rev-parse", "HEAD").trim();
  mainEdits(dirs, "dropped.rs", "a story the lane merged and later reset away\n");
  reviewed(dirs, "US-1");
  // An earlier lane merged US-1 onto main, so its tip entered main's reflog, and then reset main.
  git(dirs.repo, "merge", "-q", "--ff-only", "story/US-1");
  git(dirs.repo, "reset", "-q", "--hard", init);
  const { agent } = implAgent({});

  await deliver({ feature: "board" }, agent);

  assert.equal(dirs.story("US-1").status, "done");
  assert.equal(subjects(dirs)[0], "feat: US-1 attempt");
  assert.ok(subjects(dirs).includes("chore: main edits dropped.rs"), "the merge base replays the reset-away commit too");
});

test("a story the lane sent back keeps its branch where the lane left it and is merged again from there, so only its own commit reaches the integration branch", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const failed = join(dirs.root, "fast-failed-once");
  dirs.checks({ fast: `test -e ${failed} || { touch ${failed}; exit 1; }`, stack_up: "true", stack: "true" });
  const wt = reviewed(dirs, "US-1");
  mainEdits(dirs, "m1.rs", "one\n");
  const seen = {};
  // The implementer that continues sees the branch as the lane left it; main moves on before it is merged again.
  const { agent } = implAgent({ "US-1": async () => {
    seen.branch = git(wt, "log", "--format=%s").trim().split("\n");
    seen.base = git(dirs.repo, "config", "--get", "branch.story/US-1.omnibase").trim();
    mainEdits(dirs, "m2.rs", "two\n");
    return { status: "ready", summary: "continued" };
  } });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(seen.branch, ["feat: US-1 attempt", "chore: main edits m1.rs", "init"]);
  assert.equal(seen.base, git(dirs.repo, "rev-parse", "main~2").trim(), "the recorded base is the head the lane rebased onto");
  assert.deepEqual(subjects(dirs), ["feat: US-1 attempt", "chore: main edits m2.rs", "chore: main edits m1.rs", "init"]);
  assert.equal(dirs.story("US-1").status, "done");
  assert.deepEqual([existsSync(wt), hasBranch(dirs, "US-1")], [false, false]);
});

// Two commits on main change the same line, so replaying the first onto the
// second conflicts: a lane that rebases from too old a base cannot merge.
function mainChangesTheSameLineTwice(dirs) {
  mainEdits(dirs, "shared.rs", "two\n");
  mainEdits(dirs, "shared.rs", "three\n");
}

test("a branch an implementer rebased onto the integration branch after a conflict is merged from where it now sits, not from the base recorded when it was cut", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  writeFileSync(join(dirs.repo, "shared.rs"), "one\n");
  git(dirs.repo, "add", "shared.rs");
  git(dirs.repo, "commit", "-q", "-m", "chore: shared");
  const wt = reviewed(dirs, "US-1");
  git(dirs.repo, "config", "branch.story/US-1.omnibase", sha(dirs, "main"));
  mainChangesTheSameLineTwice(dirs);
  // The implementer rebased the branch onto main itself, as the rebase-conflict finding asks.
  git(wt, "rebase", "-q", "main");
  const { agent } = implAgent({});

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(subjects(dirs), ["feat: US-1 attempt", "chore: main edits shared.rs", "chore: main edits shared.rs", "chore: shared", "init"]);
  assert.equal(dirs.story("US-1").status, "done");
});

test("a branch rebuilt on the integration branch, whose recorded base is on neither of them, is merged from git's fork point", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  writeFileSync(join(dirs.repo, "shared.rs"), "one\n");
  git(dirs.repo, "add", "shared.rs");
  git(dirs.repo, "commit", "-q", "-m", "chore: shared");
  // The base recorded for the branch is a commit of an abandoned line.
  git(dirs.repo, "checkout", "-q", "-b", "abandoned");
  mainEdits(dirs, "old.rs", "an earlier attempt\n");
  const abandoned = sha(dirs, "HEAD");
  git(dirs.repo, "checkout", "-q", "main");
  git(dirs.repo, "branch", "-q", "-D", "abandoned");
  mainChangesTheSameLineTwice(dirs);
  reviewed(dirs, "US-1");
  git(dirs.repo, "config", "branch.story/US-1.omnibase", abandoned);
  const { agent } = implAgent({});

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(subjects(dirs), ["feat: US-1 attempt", "chore: main edits shared.rs", "chore: main edits shared.rs", "chore: shared", "init"]);
  assert.equal(dirs.story("US-1").status, "done");
});

// ---- time limits: no call, command or post waits forever --------------------------

// Time limits are in minutes; the tests use tiny ones.
const seconds = (s) => s / 60;

test("a lane command running past its time limit is stopped with everything it started, and the story is sent back with a finding that names the limit", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  const childPid = join(dirs.root, "child.pid");
  dirs.checks({ fast: "true", stack_up: "true", stack: `sleep 30 & echo $! > ${childPid}; wait` });
  reviewed(dirs, "US-1");
  const seen = {};
  const { agent } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board", timeouts: { stack: seconds(1) } }, agent, onSendBack(dirs, "US-1", seen), (fns) => Promise.all(fns.map((f) => f())), () => {});

  const [finding] = seen.sentBack.findings;
  assert.match(finding.text, /^\[attempt 1\] The stack command .* failed on the main worktree\. Output:\n/);
  assert.match(finding.text, /Timed out: the stack command ran past its limit of 1s \(args\.timeouts\.stack\) and was stopped\./);
  assert.throws(() => process.kill(Number(readFileSync(childPid, "utf8")), 0), { code: "ESRCH" }, "the command's own child was stopped with it");
});

test("a lane command that reads standard input gets none, so it ends at once instead of waiting for its time limit", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "reviewed" })]);
  dirs.checks({ fast: "cat", stack_up: "true", stack: "true" });
  reviewed(dirs, "US-1");
  const lines = [];
  const { agent } = implAgent({ "US-1": stopped });

  await workflow({ feature: "board", timeouts: { fast: seconds(5) } }, agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.ok(lines.includes("lane:US-1: fast passed"), lines.filter((l) => l.startsWith("lane:")).join("\n"));
  assert.equal(dirs.story("US-1").status, "done");
});

test("a call that hangs fails at the call limit with an error that says it timed out, so a hung post of the report leaves the run to end unposted", { timeout: 20000 }, async (t) => {
  scratch(t);
  process.env.FAKE_OMNI_HANG = "post";
  const lines = [];

  const result = await workflow({ feature: "board", timeouts: { call: seconds(2) } }, clarityAgent().agent, () => {}, (fns) => Promise.all(fns.map((f) => f())), (m) => lines.push(m));

  assert.equal(result.posted, false);
  assert.ok(lines.some((l) => /^The report could not be posted: `omni post \S+ --summary Report of run-\S+ --body-file \S+ --feature board` timed out after 2s$/.test(l)), lines.join("\n"));
});

test("the clarity question and the run's report are posted from body files, which are gone once posted", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-7")]);

  const result = await start("board", clarityAgent({ "US-7": [blockingGap] }).agent);

  const posts = dirs.state().calls.filter((c) => c.argv[0] === "post");
  assert.deepEqual(posts.map((c) => c.argv.includes("--body-file")), [true, true]);
  for (const c of posts) assert.equal(existsSync(c.argv[c.argv.indexOf("--body-file") + 1]), false, "the body file is removed after the post");
  const threads = dirs.state().threads;
  assert.match(threads.find((th) => th.title === "US-7: Which colour is the badge?").posts[0].body, /Options:\n1\. Paint it red\n2\. Paint it blue\n/);
  assert.equal(threads.find((th) => th.title.startsWith("Run ")).posts[0].body, result.report);
});

test("every agent call names its role, by which the runner sets the session's time limit", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const roles = {};
  const impl = implAgent({ "US-1": async () => ({ status: "ready", summary: "done" }) });

  await deliver({ feature: "board" }, async (prompt, opts) => {
    roles[opts.label] = opts.role;
    return impl.agent(prompt, opts);
  });

  assert.deepEqual(roles, { "clarity:US-1": "clarity", "impl:US-1": "impl", "review:US-1": "review", "verify:US-1": "verify", "tests:US-1": "tests", "final check": "wrap" });
});
