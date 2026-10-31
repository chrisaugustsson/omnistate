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

const blockingGap = { criterion: "The badge has a colour", question: "Which colour is the badge?", options: ["Paint it red", "Paint it blue"], recommendation: "Paint it red", reason: "\"warning colour\"", blocking: true };
const minorGap = (n) => ({ criterion: `Criterion ${n}`, question: `Minor question ${n}?`, options: [`Option ${n}a`, `Option ${n}b`], recommendation: `Option ${n}a`, reason: "convention", blocking: false });

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

test("claims at its stage an implementing story of another run whose worktree has a commit ahead", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implementing", holder: { run: "run-old", stage: "implementing" } })]);
  const wt = worktree(dirs, "US-1");
  writeFileSync(join(wt, "feature.txt"), "work\n");
  git(wt, "add", "feature.txt");
  git(wt, "commit", "-q", "-m", "work");

  const result = await start("board");

  assert.deepEqual(dirs.story("US-1").holder, { run: result.run, stage: "implementing" });
  assert.equal(dirs.story("US-1").status, "implementing");
  assert.equal(existsSync(wt), true);
});

test("claims an unheld review story whose worktree has a commit ahead", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review" })]);
  const wt = worktree(dirs, "US-1");
  writeFileSync(join(wt, "feature.txt"), "work\n");
  git(wt, "add", "feature.txt");
  git(wt, "commit", "-q", "-m", "work");

  const result = await start("board");

  assert.deepEqual(dirs.story("US-1").holder, { run: result.run, stage: "review" });
  assert.equal(dirs.story("US-1").status, "review");
});

test("returns an implementing story with no commits ahead to backlog unheld, force-removing its worktree and deleting its branch", async (t) => {
  const dirs = scratch(t, [story("US-2", { status: "implementing", holder: { run: "run-old", stage: "implementing" } })]);
  const wt = worktree(dirs, "US-2");
  writeFileSync(join(wt, "README"), "uncommitted\n");

  await start("board");

  assert.equal(dirs.story("US-2").status, "backlog");
  assert.equal(dirs.story("US-2").holder, null);
  assert.equal(existsSync(wt), false);
  assert.equal(git(dirs.repo, "branch", "--list", "story/US-2"), "");
});

test("returns an implementing story whose worktree is missing to backlog unheld", async (t) => {
  const dirs = scratch(t, [story("US-2", { status: "implementing", holder: { run: "run-old", stage: "implementing" } })]);

  await start("board");

  assert.equal(dirs.story("US-2").status, "backlog");
  assert.equal(dirs.story("US-2").holder, null);
});

test("keeps a testing story of another run in testing, claimed by this run", async (t) => {
  const dirs = scratch(t, [story("US-3", { status: "testing", holder: { run: "run-old", stage: "testing" } })]);

  const result = await start("board");

  assert.equal(dirs.story("US-3").status, "testing");
  assert.deepEqual(dirs.story("US-3").holder, { run: result.run, stage: "testing" });
});

test("only releases a blocked story of another run, without claiming it or moving it", async (t) => {
  const dirs = scratch(t, [story("US-4", { status: "implementing", blocked: true, holder: { run: "run-old", stage: "implementing" } })]);

  await start("board");

  assert.equal(dirs.story("US-4").holder, null);
  assert.equal(dirs.story("US-4").status, "implementing");
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

test("claims an unblocked review story at review for this run and keeps its worktree", async (t) => {
  const dirs = scratch(t, [story("US-6", { status: "review", blocked: true, holder: { run: "run-old", stage: "review" }, links: [questionLink("th-1")] })], [thread("th-1", "US-6: Which colour is the badge?", "Use option A")]);
  const wt = worktree(dirs, "US-6");
  writeFileSync(join(wt, "feature.txt"), "work\n");
  git(wt, "add", "feature.txt");
  git(wt, "commit", "-q", "-m", "work");

  const result = await start("board");

  assert.deepEqual(dirs.story("US-6").holder, { run: result.run, stage: "review" });
  assert.equal(dirs.story("US-6").status, "review");
  assert.equal(dirs.story("US-6").blocked, false);
  assert.equal(existsSync(wt), true);
});

test("claims an unblocked implementing story at implementing for this run", async (t) => {
  const dirs = scratch(t, [story("US-6", { status: "implementing", blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-6: Which colour is the badge?", "Use option A")]);

  const result = await start("board");

  assert.deepEqual(dirs.story("US-6").holder, { run: result.run, stage: "implementing" });
  assert.equal(dirs.story("US-6").status, "implementing");
});
