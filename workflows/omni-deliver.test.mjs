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

  assert.deepEqual(claimsOf(dirs, "US-1"), [{ run: result.run, stage: "implementing" }]);
  assert.equal(dirs.story("US-1").status, "implementing");
  assert.equal(existsSync(wt), true);
});

test("continues an implementing story it claimed at start in its existing worktree, keeping the commit, and moves it to review", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "implementing", holder: { run: "run-old", stage: "implementing" } })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = worktree(dirs, "US-1");
  writeFileSync(join(wt, "feature.txt"), "work\n");
  git(wt, "add", "feature.txt");
  git(wt, "commit", "-q", "-m", "earlier work");
  const { agent, prompts } = implAgent({ "US-1": async () => ({ status: "ready", summary: "done" }) });

  const result = await deliver({ feature: "board" }, agent);

  assert.equal(prompts["US-1"].length, 1, "the implementer ran once more, on the same story");
  assert.match(prompts["US-1"][0], new RegExp(wt.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")), "in the existing worktree");
  assert.match(git(join(dirs.root, "app"), "log", "--oneline"), /earlier work/, "the earlier commit reached the integration branch");
  assert.deepEqual(result.done, ["US-1"]);
  assert.equal(dirs.story("US-1").status, "done");
});

test("claims an unheld review story whose worktree has a commit ahead", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review" })]);
  const wt = worktree(dirs, "US-1");
  writeFileSync(join(wt, "feature.txt"), "work\n");
  git(wt, "add", "feature.txt");
  git(wt, "commit", "-q", "-m", "work");

  const result = await start("board");

  assert.deepEqual(claimsOf(dirs, "US-1"), [{ run: result.run, stage: "review" }]);
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
  assert.deepEqual(claimsOf(dirs, "US-3"), [{ run: result.run, stage: "testing" }]);
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

  assert.deepEqual(claimsOf(dirs, "US-6"), [{ run: result.run, stage: "review" }]);
  assert.equal(dirs.story("US-6").status, "review");
  assert.equal(dirs.story("US-6").blocked, false);
  assert.equal(existsSync(wt), true);
});

test("claims an unblocked implementing story at implementing for this run", async (t) => {
  const dirs = scratch(t, [story("US-6", { status: "implementing", blocked: true, links: [questionLink("th-1")] })], [thread("th-1", "US-6: Which colour is the badge?", "Use option A")]);

  const result = await start("board");

  assert.deepEqual(claimsOf(dirs, "US-6"), [{ run: result.run, stage: "implementing" }]);
  assert.equal(dirs.story("US-6").status, "implementing");
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
const branchOf = (wt) => git(wt, "rev-parse", "--abbrev-ref", "HEAD").trim();
const threadOf = (dirs, key) => dirs.state().threads.find((th) => th.id === dirs.story(key).links[0].thread_id);
const deferred = () => {
  let resolve;
  const promise = new Promise((r) => (resolve = r));
  return { promise, resolve };
};

test("with implementers 2 two stories are implementing in their own worktrees while the third waits, and the third is picked when one finishes", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2"), story("US-3")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const secondStarted = deferred();
  const thirdStarted = deferred();
  const seen = {};
  const { agent } = implAgent({
    "US-1": async () => {
      await secondStarted.promise;
      seen.us1 = { status: dirs.story("US-1").status, holder: dirs.story("US-1").holder, branch: branchOf(join(dirs.root, "app-stories", "US-1")) };
      seen.us2 = { status: dirs.story("US-2").status, holder: dirs.story("US-2").holder, branch: branchOf(join(dirs.root, "app-stories", "US-2")) };
      seen.us3 = { status: dirs.story("US-3").status, holder: dirs.story("US-3").holder };
      return { status: "ready", summary: "done" };
    },
    "US-2": async () => {
      secondStarted.resolve();
      await thirdStarted.promise;
      return { status: "ready", summary: "done" };
    },
    "US-3": async () => {
      thirdStarted.resolve();
      return { status: "ready", summary: "done" };
    },
  });

  const result = await deliver({ feature: "board", implementers: 2 }, agent);

  assert.deepEqual(seen.us1, { status: "implementing", holder: { run: result.run, stage: "implementing" }, branch: "story/US-1" });
  assert.deepEqual(seen.us2, { status: "implementing", holder: { run: result.run, stage: "implementing" }, branch: "story/US-2" });
  assert.deepEqual(seen.us3, { status: "backlog", holder: null });
  assert.equal(dirs.story("US-3").status, "done");
});

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

test("moves a ready story to review without running the fast command in its worktree (it runs once on the main worktree at the check), with files_changed from git diff <integration>...HEAD", async (t) => {
  const dirs = scratch(t, [story("US-1")]);
  const fastLog = join(dirs.root, "fast.log");
  dirs.checks({ fast: `pwd >> ${fastLog}`, stack_up: "true", stack: "true" });
  const wt = join(dirs.root, "app-stories", "US-1");
  let seenAtReview;
  const { agent, reviews } = implAgent({
    "US-1": async () => {
      mkdirSync(join(wt, "src"));
      writeFileSync(join(wt, "src", "board.rs"), "board\n");
      writeFileSync(join(wt, "NOTES"), "notes\n");
      git(wt, "add", "src/board.rs", "NOTES");
      git(wt, "commit", "-q", "-m", "feat: Add board");
      writeFileSync(join(dirs.repo, "OTHER"), "other\n");
      git(dirs.repo, "add", "OTHER");
      git(dirs.repo, "commit", "-q", "-m", "other work on main");
      return { status: "ready", summary: "done" };
    },
  }, {
    "US-1": () => {
      seenAtReview = dirs.story("US-1").status;
      return { verdict: "pass", findings: [] };
    },
  });

  await deliver({ feature: "board" }, agent);

  assert.equal(reviews["US-1"].length, 1);
  assert.equal(seenAtReview, "review");
  assert.equal(dirs.story("US-1").status, "done");
  assert.deepEqual(dirs.story("US-1").files_changed, ["NOTES", "src/board.rs"]);
  assert.equal(readFileSync(fastLog, "utf8"), `${dirs.repo}\n`, "the fast command ran on the main worktree only");
});

test("an implementer returning blocked leaves its story blocked in implementing, released at the run's end, with a thread holding the summary, and frees the slot for the next story", async (t) => {
  const dirs = scratch(t, [story("US-1"), story("US-2")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const { agent } = implAgent({
    "US-1": async () => ({ status: "blocked", summary: "The board route is missing from the API." }),
    "US-2": async () => ({ status: "ready", summary: "done" }),
  });

  await deliver({ feature: "board", implementers: 1 }, agent);

  const th = threadOf(dirs, "US-1");
  assert.equal(th.title, "US-1: the implementer is blocked");
  assert.ok(th.posts[0].body.includes("The board route is missing from the API."));
  assert.deepEqual(dirs.story("US-1").links, [{ thread_id: th.id, label: "question" }]);
  assert.equal(dirs.story("US-1").blocked, true);
  assert.equal(dirs.story("US-1").status, "implementing");
  assert.equal(dirs.story("US-1").holder, null);
  assert.equal(existsSync(join(dirs.root, "app-stories", "US-1")), true);
  assert.equal(dirs.story("US-2").status, "done");
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

// ---- review in the worktree -------------------------------------------------

// A story another run left in review, with its worktree one commit ahead; the run
// claims it at review and the Implement phase leaves it alone.
function inReview(dirs, key) {
  const wt = worktree(dirs, key);
  mkdirSync(join(wt, "src"));
  writeFileSync(join(wt, "src", "board.rs"), "board\n");
  git(wt, "add", "src/board.rs");
  git(wt, "commit", "-q", "-m", "feat: Add board");
  return wt;
}

// An implementer that commits a new file on each call, as a fix: commit.
const fixer = (wt, key) => async (n) => {
  writeFileSync(join(wt, "src", `fix${n}.rs`), "fix\n");
  git(wt, "add", `src/fix${n}.rs`);
  git(wt, "commit", "-q", "-m", `fix: Address review of ${key}`);
  return { status: "ready", summary: "fixed" };
};

const fail = (...texts) => ({ verdict: "fail", findings: texts.map((text) => ({ text, files: ["src/board.rs"], criterion: "The badge has a colour" })) });
const openFindings = (s) => (s.findings || []).filter((f) => f.stage === "review" && !f.resolved).map((f) => f.text);

test("a story that fails review three times is blocked in review with a thread listing the third round's findings, and no third implementer runs", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = inReview(dirs, "US-1");
  const { agent, prompts, reviews } = implAgent({ "US-1": fixer(wt, "US-1") }, { "US-1": (n) => fail(`Round ${n}: the badge has no colour`) });

  await deliver({ feature: "board" }, agent);

  assert.equal(reviews["US-1"].length, 3);
  assert.equal(prompts["US-1"].length, 2);
  const th = threadOf(dirs, "US-1");
  assert.equal(th.title, "US-1: the review still fails after three attempts");
  assert.ok(th.posts[0].body.includes("Round 3: the badge has no colour"));
  assert.ok(!th.posts[0].body.includes("Round 1") && !th.posts[0].body.includes("Round 2"));
  assert.deepEqual(dirs.story("US-1").links, [{ thread_id: th.id, label: "question" }]);
  assert.equal(dirs.story("US-1").blocked, true);
  assert.equal(dirs.story("US-1").status, "review");
  assert.equal(dirs.story("US-1").holder, null);
  assert.deepEqual(openFindings(dirs.story("US-1")), ["Round 3: the badge has no colour"]);
});

test("a review fail with two findings is fixed in the worktree with no fast command there, files_changed is rewritten, and a passing second review resolves the findings and moves it to testing", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review", files_changed: ["src/board.rs"] })]);
  const marker = join(dirs.root, "fast-runs");
  dirs.checks({ fast: `git cat-file -e HEAD:src/fix1.rs && pwd >> ${marker}`, stack_up: "true", stack: "true" });
  const wt = inReview(dirs, "US-1");
  const seen = {};
  const fix = fixer(wt, "US-1");
  const { agent, prompts, reviews } = implAgent({
    "US-1": async (n) => {
      seen.status = dirs.story("US-1").status;
      seen.open = openFindings(dirs.story("US-1"));
      return fix(n);
    },
  }, { "US-1": (n) => (n === 1 ? fail("The badge has no colour", "The badge count shows zero") : { verdict: "pass", findings: [] }) });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(seen, { status: "review", open: ["The badge has no colour", "The badge count shows zero"] });
  assert.ok(prompts["US-1"][0].includes("The badge has no colour") && prompts["US-1"][0].includes("The badge count shows zero"));
  assert.ok(prompts["US-1"][0].includes(wt));
  assert.equal(readFileSync(marker, "utf8"), `${dirs.repo}\n`, "the fast command ran on the main worktree only");
  assert.deepEqual(dirs.story("US-1").files_changed, ["src/board.rs", "src/fix1.rs"]);
  assert.equal(reviews["US-1"].length, 2);
  assert.deepEqual(openFindings(dirs.story("US-1")), []);
  assert.equal(dirs.story("US-1").findings.length, 2);
  assert.equal(dirs.story("US-1").status, "done");
});

test("a reviewer returning needs_decision blocks the story in review with a thread 'US-<n>: <question>' holding the options and recommendation, and no implementer runs", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = inReview(dirs, "US-1");
  const { agent, prompts } = implAgent({ "US-1": fixer(wt, "US-1") }, {
    "US-1": () => ({ verdict: "needs_decision", findings: [], question: "Which colour is the badge?", options: ["Paint it red", "Paint it blue"], recommendation: "Paint it red" }),
  });

  await deliver({ feature: "board" }, agent);

  const th = threadOf(dirs, "US-1");
  assert.equal(th.title, "US-1: Which colour is the badge?");
  assert.ok(th.posts[0].body.includes("Paint it red") && th.posts[0].body.includes("Paint it blue"));
  assert.ok(th.posts[0].body.includes("Recommendation: Paint it red"));
  assert.deepEqual(dirs.story("US-1").links, [{ thread_id: th.id, label: "question" }]);
  assert.equal(dirs.story("US-1").blocked, true);
  assert.equal(dirs.story("US-1").status, "review");
  assert.equal(prompts["US-1"], undefined);
});

test("with reviewers 2 two stories in review are reviewed at the same time", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "review" }), story("US-2", { status: "review" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inReview(dirs, "US-1");
  inReview(dirs, "US-2");
  const secondStarted = deferred();
  const { agent } = implAgent({}, {
    "US-1": async () => {
      await secondStarted.promise;
      return { verdict: "pass", findings: [] };
    },
    "US-2": async () => {
      secondStarted.resolve();
      return { verdict: "pass", findings: [] };
    },
  });

  await deliver({ feature: "board", reviewers: 2 }, agent);

  assert.equal(dirs.story("US-1").status, "done");
  assert.equal(dirs.story("US-2").status, "done");
});

// ---- merge on the main worktree ---------------------------------------------

// A story another run left in testing, its worktree one commit ahead with the
// file given; the run claims it at testing and the merge worker takes it.
function inTesting(dirs, key, file, content) {
  const wt = worktree(dirs, key);
  mkdirSync(join(wt, "src"), { recursive: true });
  writeFileSync(join(wt, file), content);
  git(wt, "add", file);
  git(wt, "commit", "-q", "-m", `feat: ${key}`);
  return wt;
}

const subjects = (dirs) => git(dirs.repo, "log", "--format=%s", "main").trim().split("\n");

// A main-worktree commit to README, so a story that also changed README conflicts.
function mainEditsReadme(dirs) {
  writeFileSync(join(dirs.repo, "README"), "app\nfrom main\n");
  git(dirs.repo, "commit", "-q", "-am", "chore: main edit");
}

// An implementer that rebases the worktree onto main and resolves README.
const rebaser = (wt) => {
  try {
    git(wt, "rebase", "main");
  } catch {}
  writeFileSync(join(wt, "README"), "app\nfrom main\nfrom US-1\n");
  git(wt, "add", "README");
  execFileSync("git", ["rebase", "--continue"], { cwd: wt, stdio: "ignore", env: { ...process.env, GIT_EDITOR: "true" } });
  return { status: "ready", summary: "rebased" };
};

test("two testing stories where the lower-numbered one is blocked by the other merge blocker first, each by a fast-forward, with stack_up run once before the first", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", blocked_by: ["US-2"] }), story("US-2", { status: "testing" })]);
  const upLog = join(dirs.root, "up.log");
  dirs.checks({ fast: "true", stack_up: `git log --format=%s -1 >> ${upLog}`, stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");

  await deliver({ feature: "board" }, implAgent({}).agent);

  assert.equal(readFileSync(upLog, "utf8"), "init\n");
  assert.deepEqual(subjects(dirs), ["feat: US-1", "feat: US-2", "init"]);
  assert.equal(git(dirs.repo, "rev-list", "--merges", "main").trim(), "");
});

test("testing stories with no blockers between them merge by priority before number", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", priority: 3 }), story("US-2", { status: "testing", priority: 1 })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");

  await deliver({ feature: "board" }, implAgent({}).agent);

  assert.deepEqual(subjects(dirs), ["feat: US-1", "feat: US-2", "init"]);
});

test("a rebase conflict sends the story to implementing with a finding naming the file, an implementer rebases it in its worktree with no fast command there, and it returns to testing unreviewed and is fast-forwarded before the next story, resolving the finding", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", files_changed: ["src/stale.rs"] }), story("US-2", { status: "testing" })]);
  const fastLog = join(dirs.root, "fast.log");
  dirs.checks({ fast: `pwd >> ${fastLog}`, stack_up: "true", stack: "true" });
  const wt = inTesting(dirs, "US-1", "README", "app\nfrom US-1\n");
  writeFileSync(join(wt, "src", "a.rs"), "a\n");
  git(wt, "add", "src/a.rs");
  git(wt, "commit", "-q", "-m", "feat: US-1 code");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  mainEditsReadme(dirs);
  const seen = {};
  const { agent, prompts, reviews } = implAgent({
    "US-1": async () => {
      const s = dirs.story("US-1");
      Object.assign(seen, { status: s.status, findings: s.findings, main: subjects(dirs)[0] });
      return rebaser(wt);
    },
  });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(seen, { status: "implementing", findings: [{ stage: "testing", text: "rebase conflict in README", files: ["README"], resolved: false }], main: "chore: main edit" });
  assert.equal(prompts["US-1"].length, 1);
  assert.ok(prompts["US-1"][0].includes(wt));
  assert.ok(prompts["US-1"][0].includes("git rebase main"));
  assert.equal(readFileSync(fastLog, "utf8"), `${dirs.repo}\n${dirs.repo}\n`);
  assert.deepEqual(dirs.story("US-1").files_changed, ["README", "src/a.rs"]);
  assert.equal(reviews["US-1"], undefined);
  assert.equal(dirs.story("US-1").status, "done");
  assert.deepEqual(dirs.story("US-1").findings, [{ stage: "testing", text: "rebase conflict in README", files: ["README"], resolved: true }]);
  assert.deepEqual(subjects(dirs), ["feat: US-2", "feat: US-1 code", "feat: US-1", "chore: main edit", "init"]);
  assert.equal(git(dirs.repo, "rev-list", "--merges", "main").trim(), "");
});

test("a rebase implementer returning blocked blocks the story in implementing with a thread, and no other testing story merges in the run", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" }), story("US-2", { status: "testing" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "README", "app\nfrom US-1\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  mainEditsReadme(dirs);
  const { agent } = implAgent({ "US-1": async () => ({ status: "blocked", summary: "README cannot be merged by hand" }) });

  await deliver({ feature: "board" }, agent);

  const th = threadOf(dirs, "US-1");
  assert.ok(th.posts[0].body.includes("README cannot be merged by hand"));
  assert.equal(dirs.story("US-1").blocked, true);
  assert.equal(dirs.story("US-1").status, "implementing");
  assert.equal(dirs.story("US-2").status, "testing");
  assert.deepEqual(subjects(dirs), ["chore: main edit", "init"]);
});

test("while the rebase implementer works, with implementers 1 a pickable story is still picked and implemented", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" }), story("US-3")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  const wt = inTesting(dirs, "US-1", "README", "app\nfrom US-1\n");
  mainEditsReadme(dirs);
  const implStarted = deferred();
  const seen = {};
  const { agent } = implAgent({
    "US-1": async () => {
      seen.picked = await Promise.race([implStarted.promise.then(() => true), new Promise((r) => setTimeout(() => r(false), 5000))]);
      return rebaser(wt);
    },
    "US-3": async () => {
      implStarted.resolve();
      const wt3 = join(dirs.root, "app-stories", "US-3");
      writeFileSync(join(wt3, "c.rs"), "c\n");
      git(wt3, "add", "c.rs");
      git(wt3, "commit", "-q", "-m", "feat: US-3");
      return { status: "ready", summary: "done" };
    },
  });

  await deliver({ feature: "board", implementers: 1 }, agent);

  assert.equal(seen.picked, true);
  assert.equal(dirs.story("US-3").status, "done");
});

test("a stack_up exiting non-zero merges nothing from the batch and posts its output on a thread on the feature", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" })]);
  dirs.checks({ fast: "true", stack_up: "echo the stack broke; exit 1", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");

  await deliver({ feature: "board" }, implAgent({}).agent);

  assert.deepEqual(subjects(dirs), ["init"]);
  const posted = dirs.state().threads.filter((th) => th.feature === "board" && th.posts.some((p) => p.body.includes("the stack broke")));
  assert.equal(posted.length > 0, true);
});

// ---- check the merged story, and the fix story --------------------------------

// A verifier reply for the story's criteria: every one ok except the indexes in
// failing, whose evidence is the text given.
const verdict = (dirs, key, failing = {}) => ({
  criteria: dirs.story(key).criteria.map((c, i) => ({ index: i, criterion: c.text || c, ok: !(i in failing), evidence: failing[i] ?? "ran it" })),
});

// An implementer that commits the file given in the story's worktree.
const committer = (dirs, key, file, before = () => {}) => async () => {
  before();
  const wt = join(dirs.root, "app-stories", key);
  mkdirSync(join(wt, "src"), { recursive: true });
  writeFileSync(join(wt, file), `${key}\n`);
  git(wt, "add", file);
  git(wt, "commit", "-q", "-m", `feat: ${key}`);
  return { status: "ready", summary: "fixed" };
};

const testingFindings = (s) => (s.findings || []).filter((f) => f.stage === "testing").map(({ text, resolved }) => ({ text, resolved }));
const hasBranch = (dirs, key) => git(dirs.repo, "branch", "--list", `story/${key}`).trim() !== "";
const LINT = "Typecheck/lint passes (make lint)";

test("a merged story whose criteria all pass has each criterion ticked, the human check pending, its worktree and branch gone, and the fast command ran on the main worktree", async (t) => {
  const criteria = [{ text: "The badge has a colour", human_check: false }, { text: "The board shows the badge. Verify in browser using dev-browser skill", human_check: true }];
  const dirs = scratch(t, [story("US-1", { status: "testing", criteria })]);
  const fastLog = join(dirs.root, "fast.log");
  dirs.checks({ fast: `pwd >> ${fastLog}`, stack_up: "true", stack: "true" });
  const wt = inTesting(dirs, "US-1", "src/a.rs", "a\n");

  await deliver({ feature: "board" }, implAgent({}, {}, { "US-1": () => verdict(dirs, "US-1") }).agent);

  assert.deepEqual(dirs.story("US-1").criteria, [{ ...criteria[0], ticked: true }, { ...criteria[1], ticked: true, pending: true }]);
  assert.equal(existsSync(wt), false);
  assert.equal(hasBranch(dirs, "US-1"), false);
  assert.equal(readFileSync(fastLog, "utf8"), `${dirs.repo}\n`);
  assert.deepEqual(subjects(dirs), ["feat: US-1", "init"]);
});

test("a failed criterion becomes a priority-0 fix story that merges before the rest of the batch, then the original passes its re-check and goes on with its testing findings resolved", async (t) => {
  const links = [{ doc_id: "d1", node_id: "n1", label: "spec" }, { thread_id: "t-old", label: "question" }];
  const dirs = scratch(t, [
    story("US-1", { status: "testing", criteria: ["The badge has a colour", "The badge shows the count"], files_changed: ["src/a.rs"], links }),
    story("US-2", { status: "testing" }),
  ]);
  const upLog = join(dirs.root, "up.log");
  const stackLog = join(dirs.root, "stack.log");
  dirs.checks({ fast: "true", stack_up: `git log --format=%s -1 >> ${upLog}`, stack: `git log --format=%s -1 >> ${stackLog}` });
  const wt = inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  const seen = {};
  const { agent, verifies } = implAgent(
    { "US-3": committer(dirs, "US-3", "src/fix.rs", () => Object.assign(seen, { status: dirs.story("US-1").status, findings: testingFindings(dirs.story("US-1")), worktree: existsSync(wt), branch: hasBranch(dirs, "US-1"), main: subjects(dirs) })) },
    {},
    { "US-1": (n) => verdict(dirs, "US-1", n === 1 ? { 1: "the count is missing" } : {}), "US-3": () => verdict(dirs, "US-3") },
  );

  await deliver({ feature: "board" }, agent);

  const fix = dirs.story("US-3");
  assert.ok(fix.title.startsWith("Fix US-1: "));
  assert.equal(fix.priority, 0);
  assert.ok(fix.text.includes("the count is missing"));
  assert.deepEqual(fix.files_planned, ["src/a.rs"]);
  assert.deepEqual(fix.criteria.map((c) => c.text), ["The badge shows the count", LINT]);
  assert.deepEqual(fix.links, [{ doc_id: "d1", node_id: "n1", label: "spec" }]);
  assert.equal(seen.status, "testing");
  assert.equal(seen.findings.length, 2);
  assert.ok(seen.findings[0].text.includes("The badge shows the count") && seen.findings[0].text.includes("the count is missing"));
  assert.deepEqual(seen.findings[1], { text: "waiting for US-3", resolved: false });
  assert.equal(seen.findings[0].resolved, false);
  assert.deepEqual([seen.worktree, seen.branch], [true, true]);
  assert.deepEqual(seen.main, ["feat: US-1", "init"]);
  assert.deepEqual(subjects(dirs), ["feat: US-2", "feat: US-3", "feat: US-1", "init"]);
  assert.equal(verifies["US-1"], 2);
  assert.deepEqual(dirs.story("US-1").criteria.map((c) => c.ticked), [true, true]);
  assert.deepEqual(testingFindings(dirs.story("US-1")).map((f) => f.resolved), [true, true]);
  assert.equal(dirs.story("US-1").blocked, false);
  assert.equal(existsSync(wt), false);
  assert.equal(hasBranch(dirs, "US-1"), false);
  assert.equal(readFileSync(upLog, "utf8"), "init\n");
  assert.equal(readFileSync(stackLog, "utf8"), "feat: US-2\n");
});

test("an original still failing after its fix story is blocked with a thread holding the new findings, only the waiting finding is resolved, and the fix story and the next story go on", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", criteria: ["The badge shows the count"] }), story("US-2", { status: "testing" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  const fixWt = join(dirs.root, "app-stories", "US-3");
  const { agent } = implAgent(
    { "US-3": committer(dirs, "US-3", "src/fix.rs") },
    {},
    { "US-1": (n) => verdict(dirs, "US-1", { 0: `round ${n}: the count is missing` }), "US-3": () => verdict(dirs, "US-3") },
  );

  await deliver({ feature: "board" }, agent);

  const original = dirs.story("US-1");
  assert.equal(original.blocked, true);
  assert.equal(original.status, "testing");
  const found = testingFindings(original);
  assert.equal(found.length, 3);
  assert.ok(found[0].text.includes("round 1: the count is missing") && !found[0].resolved);
  assert.deepEqual(found[1], { text: "waiting for US-3", resolved: true });
  assert.ok(found[2].text.includes("round 2: the count is missing") && !found[2].resolved);
  const th = threadOf(dirs, "US-1");
  assert.ok(th.title.startsWith("US-1: "));
  assert.ok(th.posts[0].body.includes("round 2: the count is missing"));
  assert.deepEqual(dirs.state().stories.map((s) => s.key), ["US-1", "US-2", "US-3"]);
  assert.deepEqual(dirs.story("US-3").criteria.map((c) => c.ticked), [true, true]);
  assert.equal(existsSync(fixWt), false);
  assert.deepEqual(subjects(dirs), ["feat: US-2", "feat: US-3", "feat: US-1", "init"]);
});

test("a fix story failing its own check blocks it and the original with one thread named after the original holding both findings, makes no second fix story, and a third story merges afterwards", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", criteria: ["The badge shows the count"] }), story("US-2", { status: "testing" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  const { agent } = implAgent(
    { "US-3": committer(dirs, "US-3", "src/fix.rs") },
    {},
    { "US-1": () => verdict(dirs, "US-1", { 0: "the count is missing" }), "US-3": () => verdict(dirs, "US-3", { 0: "the fix shows no count either" }) },
  );

  await deliver({ feature: "board" }, agent);

  assert.equal(dirs.story("US-1").blocked, true);
  assert.equal(dirs.story("US-3").blocked, true);
  const threadIds = (key) => dirs.story(key).links.filter((l) => l.thread_id).map((l) => l.thread_id);
  assert.equal(threadIds("US-1").length, 1);
  assert.deepEqual(threadIds("US-3"), threadIds("US-1"));
  const th = threadOf(dirs, "US-1");
  assert.ok(th.title.startsWith("US-1: "));
  assert.ok(th.posts[0].body.includes("the count is missing") && th.posts[0].body.includes("the fix shows no count either"));
  assert.ok(testingFindings(dirs.story("US-3")).some((f) => f.text.includes("the fix shows no count either") && !f.resolved));
  assert.deepEqual(dirs.state().stories.map((s) => s.key), ["US-1", "US-2", "US-3"]);
  assert.deepEqual(subjects(dirs), ["feat: US-2", "feat: US-3", "feat: US-1", "init"]);
});

test("after a batch of two the stack command runs once, and its failure makes one priority-0 fix story 'Fix batch US-2, US-1' in merge order with the output, both stories' files and links, which is merged and checked", async (t) => {
  const dirs = scratch(t, [
    story("US-1", { status: "testing", files_changed: ["src/a.rs"], links: [{ doc_id: "d1", node_id: "n1", label: "" }] }),
    story("US-2", { status: "testing", priority: 1, files_changed: ["src/b.rs"], links: [{ doc_id: "d2", node_id: "n2", label: "" }] }),
  ]);
  const stackLog = join(dirs.root, "stack.log");
  dirs.checks({ fast: "true", stack_up: "true", stack: `git log --format=%s -1 >> ${stackLog}; echo the stack broke; exit 1` });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  const { agent, verifies } = implAgent({ "US-3": committer(dirs, "US-3", "src/fix.rs") }, {}, { "US-3": () => verdict(dirs, "US-3") });

  await deliver({ feature: "board" }, agent);

  assert.equal(readFileSync(stackLog, "utf8"), "feat: US-1\n");
  const fix = dirs.story("US-3");
  assert.equal(fix.title, "Fix batch US-2, US-1");
  assert.equal(fix.priority, 0);
  assert.ok(fix.text.includes("the stack broke"));
  assert.deepEqual(fix.criteria.map((c) => c.text), ["The stack command passes on the main worktree", LINT]);
  assert.deepEqual(fix.files_planned, ["src/b.rs", "src/a.rs"]);
  assert.deepEqual(fix.links, [{ doc_id: "d2", node_id: "n2", label: "" }, { doc_id: "d1", node_id: "n1", label: "" }]);
  assert.deepEqual(dirs.state().stories.map((s) => s.key), ["US-1", "US-2", "US-3"]);
  assert.deepEqual(subjects(dirs), ["feat: US-3", "feat: US-1", "feat: US-2", "init"]);
  assert.equal(verifies["US-3"], 1);
});

test("a batch fix story failing its own check is blocked alone, with a thread holding its findings and the stack output", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" }), story("US-2", { status: "testing" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "echo the stack broke; exit 1" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  const { agent } = implAgent({ "US-3": committer(dirs, "US-3", "src/fix.rs") }, {}, { "US-3": () => verdict(dirs, "US-3", { 0: "the stack still breaks" }) });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(dirs.state().stories.map((s) => [s.key, s.blocked]), [["US-1", false], ["US-2", false], ["US-3", true]]);
  const th = threadOf(dirs, "US-3");
  assert.ok(th.posts[0].body.includes("the stack broke") && th.posts[0].body.includes("the stack still breaks"));
});

test("a merged story whose criteria pass but whose fast command fails on the main worktree gets a testing finding with the output and a fix story for the fast command", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", criteria: ["The badge has a colour"] })]);
  const flag = join(dirs.root, "fast-broken");
  writeFileSync(flag, "");
  dirs.checks({ fast: `[ ! -e ${flag} ] || { echo fast broke on main; exit 1; }`, stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  const seen = {};
  const { agent } = implAgent(
    { "US-2": committer(dirs, "US-2", "src/fix.rs", () => { seen.findings = testingFindings(dirs.story("US-1")); rmSync(flag); }) },
    {},
    { "US-1": () => verdict(dirs, "US-1"), "US-2": () => verdict(dirs, "US-2") },
  );

  await deliver({ feature: "board" }, agent);

  assert.equal(seen.findings.length, 2);
  assert.ok(seen.findings[0].text.includes("fast broke on main"));
  assert.deepEqual(seen.findings[1], { text: "waiting for US-2", resolved: false });
  const fix = dirs.story("US-2");
  assert.equal(fix.priority, 0);
  assert.ok(fix.text.includes("fast broke on main"));
  assert.deepEqual(fix.criteria.map((c) => c.text), ["The fast command passes on the main worktree", LINT]);
});

test("a story failing a criterion and the fast command gets one fix story with the failed criterion, then the fast criterion, then the lint line", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing", criteria: ["The badge has a colour", "The badge shows the count"] })]);
  dirs.checks({ fast: `[ "$(pwd -P)" != "${dirs.repo}" ] || { echo fast broke on main; exit 1; }`, stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  const { agent } = implAgent({ "US-2": committer(dirs, "US-2", "src/fix.rs") }, {}, { "US-1": () => verdict(dirs, "US-1", { 1: "the count is missing" }), "US-2": () => verdict(dirs, "US-2") });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(dirs.state().stories.map((s) => s.key), ["US-1", "US-2"]);
  assert.deepEqual(dirs.story("US-2").criteria.map((c) => c.text), ["The badge shows the count", "The fast command passes on the main worktree", LINT]);
});

// ---- regression tests, run end and the report ---------------------------------

// A test author that commits a test file on the main worktree and reports it added.
const testWriter = (dirs, file, name, before = () => {}) => () => {
  before();
  mkdirSync(join(dirs.repo, "tests"), { recursive: true });
  writeFileSync(join(dirs.repo, "tests", file), "test\n");
  git(dirs.repo, "add", `tests/${file}`);
  git(dirs.repo, "commit", "-q", "-m", `test: Add ${name}`);
  return { ...noTests, commit: "abc", added: [{ test: name, file: `tests/${file}`, risk: "r", overlapCheck: "o", sensitivity: "s" }] };
};

// The run's report: the last post on the thread `Run <id>`, cut into its sections.
const reportOf = (dirs, run) => {
  const posts = dirs.state().threads.find((th) => th.title === `Run ${run}`).posts;
  return Object.fromEntries(posts[posts.length - 1].body.split("\n## ").slice(1).map((part) => [part.split("\n")[0], part.slice(part.indexOf("\n") + 1)]));
};

const until = async (condition) => {
  while (!condition()) await new Promise((r) => setTimeout(r, 20));
};

test("when storyLimit is reached partway through a batch, the stack command still runs once on the story merged before the stop, and the next testing story stays unmerged and released", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" }), story("US-4", { status: "testing" })]);
  const stackLog = join(dirs.root, "stack.log");
  dirs.checks({ fast: "true", stack_up: "true", stack: `git log --format=%s -1 >> ${stackLog}` });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-4", "src/d.rs", "d\n");

  await deliver({ feature: "board", storyLimit: 1 }, implAgent({}).agent);

  assert.equal(readFileSync(stackLog, "utf8"), "feat: US-1\n");
  assert.equal(dirs.story("US-1").status, "done");
  assert.deepEqual(subjects(dirs), ["feat: US-1", "init"]);
  assert.equal(dirs.story("US-4").status, "testing");
  assert.equal(dirs.story("US-4").holder, null);
});

test("a checked story's test: commit lands on main while it is still in testing, and the next story of the batch merges only after it is done; a story whose author adds nothing is done with no test: commit", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" }), story("US-2", { status: "testing" })]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  inTesting(dirs, "US-2", "src/b.rs", "b\n");
  const seen = {};
  const author = testWriter(dirs, "badge.rs", "badge test", () => Object.assign(seen, { status: dirs.story("US-1").status, main: subjects(dirs) }));
  const { agent } = implAgent({}, {}, {}, { "US-1": author });

  await deliver({ feature: "board" }, agent);

  assert.deepEqual(seen, { status: "testing", main: ["feat: US-1", "init"] });
  assert.deepEqual(subjects(dirs), ["feat: US-2", "test: Add badge test", "feat: US-1", "init"]);
  assert.equal(dirs.story("US-1").status, "done");
  assert.equal(dirs.story("US-2").status, "done");
});

test("storyLimit 1 stops picking once one story is done, lets the story in implementing reach review and stop there released, and the report's Remaining says the run stopped at the limit", { timeout: 20000 }, async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" }), story("US-2"), story("US-3")]);
  dirs.checks({ fast: "true", stack_up: "true", stack: "true" });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  const commit = committer(dirs, "US-2", "src/b.rs");
  const { agent, prompts, reviews } = implAgent({ "US-2": async () => { await until(() => dirs.story("US-1").status === "done"); return commit(); } });

  const result = await deliver({ feature: "board", implementers: 1, storyLimit: 1 }, agent);

  assert.equal(dirs.story("US-1").status, "done");
  assert.equal(dirs.story("US-2").status, "review");
  assert.equal(dirs.story("US-2").holder, null);
  assert.equal(reviews["US-2"], undefined);
  assert.equal(dirs.story("US-3").status, "backlog");
  assert.equal(prompts["US-3"], undefined);
  const remaining = reportOf(dirs, result.run).Remaining;
  assert.match(remaining, /storyLimit 1/);
  assert.match(remaining, /^- US-2\b.*\breview$/m);
  assert.match(remaining, /^- US-3\b.*\bbacklog$/m);
});

test("with one story blocked, one waiting on it and the rest done, the run ends without its lock and the Run thread's report names the done stories with commits, the blocked one with its thread, the waiting one, the fix story, the pending human check, the tests added and the journeys", async (t) => {
  const human = { text: "The board shows the badge. Verify in browser using dev-browser skill", human_check: true };
  const dirs = scratch(t, [
    story("US-1", { status: "testing", criteria: [human] }),
    story("US-2", { blocked: true, links: [questionLink("th-1")] }),
    story("US-3", { blocked_by: ["US-2"] }),
  ], [thread("th-1", "US-2: Which colour is the badge?")]);
  const once = join(dirs.root, "stack-failed");
  dirs.checks({ fast: "true", stack_up: "true", stack: `test -f ${once} || { touch ${once}; echo the stack broke; exit 1; }` });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  const { agent, prompts } = implAgent({ "US-4": committer(dirs, "US-4", "src/fix.rs") }, {}, { "US-1": () => verdict(dirs, "US-1"), "US-4": () => verdict(dirs, "US-4") }, { "US-1": testWriter(dirs, "badge.rs", "badge test") });

  const result = await deliver({ feature: "board" }, agent);

  assert.equal(existsSync(dirs.lock), false);
  assert.equal(prompts["US-3"], undefined);
  const report = reportOf(dirs, result.run);
  assert.match(report.Done, /^- US-1\b.*feat: US-1.*test: Add badge test/m);
  assert.match(report.Done, /^- US-4\b.*feat: US-4/m);
  assert.match(report.Blocked, /^- US-2\b.*th-1/m);
  assert.match(report["Waiting on dependencies"], /^- US-3\b.*US-2/m);
  assert.match(report["Fix stories created"], /^- US-4 Fix batch US-1$/m);
  assert.ok(report["Pending human checks"].includes(`US-1: ${human.text}`));
  assert.ok(report.Tests.includes("badge test"));
  assert.ok(report.Journeys.includes("Board to badge: worked"));
});

test("with storyLimit 1 reached, a failing batch stack check still drives its priority-0 fix story to done", async (t) => {
  const dirs = scratch(t, [story("US-1", { status: "testing" })]);
  const once = join(dirs.root, "stack-failed");
  dirs.checks({ fast: "true", stack_up: "true", stack: `test -f ${once} || { touch ${once}; echo the stack broke; exit 1; }` });
  inTesting(dirs, "US-1", "src/a.rs", "a\n");
  const { agent } = implAgent({ "US-2": committer(dirs, "US-2", "src/fix.rs") }, {}, { "US-2": () => verdict(dirs, "US-2") });

  await deliver({ feature: "board", storyLimit: 1 }, agent);

  assert.equal(dirs.story("US-2").title, "Fix batch US-1");
  assert.equal(dirs.story("US-2").status, "done");
  assert.deepEqual(subjects(dirs), ["feat: US-2", "feat: US-1", "init"]);
});
