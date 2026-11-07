// The Node runner of omni-deliver: its `agent` against a fake `claude`
// (fixtures/fake-claude.mjs), and its argument parsing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync, mkdirSync, chmodSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { makeAgent, parseArgs, phaseLogger, timeoutsFrom } from "./run-omni-deliver.mjs";

const fakeClaude = fileURLToPath(new URL("./fixtures/fake-claude.mjs", import.meta.url));
// Time limits are in minutes; the tests use tiny ones.
const seconds = (s) => s / 60;
// True once the process is gone; a killed child can take a moment to be reaped.
async function gone(pid) {
  for (let i = 0; i < 40; i++) {
    try { process.kill(pid, 0); } catch (e) { if (e.code === "ESRCH") return true; }
    await new Promise((r) => setTimeout(r, 50));
  }
  return false;
}

function scratch(t, reply, exit) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "run-omni-deliver-")));
  const bin = join(root, "claude");
  writeFileSync(bin, `#!/bin/sh\nexec "${process.execPath}" "${fakeClaude}" "$@"\n`);
  chmodSync(bin, 0o755);
  const logPath = join(root, "calls.jsonl");
  const runDir = join(root, "run");
  mkdirSync(runDir);
  const agentsDir = join(root, "agents");
  mkdirSync(agentsDir);
  writeFileSync(join(agentsDir, "verifier.md"), "---\nname: verifier\ntools: Bash, Read, Grep\n---\n\nYou check work and report.\n");
  writeFileSync(join(agentsDir, "implementer.md"), "---\nname: implementer\ntools: Bash, Read, Edit, Write\n---\n\nYou carry out one task.\n");
  const env = { ...process.env, OMNI_DELIVER_CLAUDE: bin, FAKE_CLAUDE_LOG: logPath };
  if (reply) env.FAKE_CLAUDE_REPLY = JSON.stringify(reply);
  if (exit) env.FAKE_CLAUDE_EXIT = String(exit);
  delete env.OMNI_DELIVER_PERMISSION_MODE;
  delete env.OMNI_DELIVER_MAX_TURNS;
  delete env.OMNI_DELIVER_TIMEOUTS;
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const calls = () => readFileSync(logPath, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  return { root, runDir, agentsDir, env, calls };
}

test("a schema call gives claude the prompt on stdin, the schema, model and agent flags, reads its stream of events and returns the structured output of the result event", async (t) => {
  const result = { type: "result", is_error: false, result: '{"gaps":[]}', structured_output: { gaps: [] }, num_turns: 3, permission_denials: [] };
  const s = scratch(t, result);
  const lines = [];
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, addDirs: ["/tmp/app-stories"], env: s.env, agentsDir: s.agentsDir, log: (m) => lines.push(m) });
  const schema = { type: "object", properties: { gaps: { type: "array" } }, required: ["gaps"] };
  const out = await agent("Check ONE story.", { schema, label: "clarity:US-1", model: "opus", agentType: "verifier" });
  assert.deepEqual(out, { gaps: [] });
  const [call] = s.calls();
  assert.equal(call.prompt, "Check ONE story.");
  assert.equal(call.cwd, s.root);
  const argv = call.argv;
  assert.deepEqual(argv.slice(0, 4), ["-p", "--output-format", "stream-json", "--verbose"]);
  assert.equal(argv[argv.indexOf("--permission-mode") + 1], "auto");
  assert.deepEqual(JSON.parse(argv[argv.indexOf("--json-schema") + 1]), schema);
  assert.equal(argv[argv.indexOf("--model") + 1], "opus");
  assert.ok(!argv.includes("--agent"), "a custom agent would hide the structured output tool");
  const systemFile = argv[argv.indexOf("--append-system-prompt-file") + 1];
  assert.equal(readFileSync(systemFile, "utf8"), "You check work and report.\n", "the agent's body without its front matter");
  assert.equal(argv[argv.indexOf("--disallowedTools") + 1], "Edit,Write,MultiEdit,NotebookEdit", "a read-only agent may not edit");
  assert.equal(argv[argv.indexOf("--add-dir") + 1], "/tmp/app-stories");
  assert.ok(existsSync(join(s.runDir, "01-clarity_US-1.prompt.md")), "the prompt is kept in the run directory");
  assert.deepEqual(JSON.parse(readFileSync(join(s.runDir, "01-clarity_US-1.reply.json"), "utf8")), result, "the reply kept in the run directory is the result event");
  assert.match(lines.at(-1), /^clarity:US-1: done in \d+s, 3 turns$/);
});

test("without a schema the agent returns claude's final text, and no schema, model or agent flag is passed", async (t) => {
  const s = scratch(t, { type: "result", is_error: false, result: "All good.", num_turns: 1 });
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, env: s.env, agentsDir: s.agentsDir });
  assert.equal(await agent("Say something.", { label: "plain" }), "All good.");
  const [call] = s.calls();
  for (const flag of ["--json-schema", "--model", "--append-system-prompt-file", "--disallowedTools", "--max-turns"]) assert.ok(!call.argv.includes(flag), `${flag} is not passed`);
});

test("an editing agent gets its instructions and no tool restriction", async (t) => {
  const s = scratch(t, { type: "result", is_error: false, result: "{}", structured_output: {}, num_turns: 1 });
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, env: s.env, agentsDir: s.agentsDir });
  await agent("Build.", { label: "impl:US-1", schema: { type: "object", properties: {} }, agentType: "implementer" });
  const [call] = s.calls();
  assert.equal(readFileSync(call.argv[call.argv.indexOf("--append-system-prompt-file") + 1], "utf8"), "You carry out one task.\n");
  assert.ok(!call.argv.includes("--disallowedTools"));
});

test("a session that exits non-zero, or reports is_error, returns null and the log says why", async (t) => {
  const failing = scratch(t, null, 2);
  const lines = [];
  const agent = makeAgent({ cwd: failing.root, runDir: failing.runDir, env: failing.env, log: (m) => lines.push(m) });
  assert.equal(await agent("Do it.", { label: "impl:US-2", schema: { type: "object", properties: {} } }), null);
  assert.match(lines.at(-1), /^impl:US-2: failed after \d+s: fake claude: refusing as told$/);

  const errored = scratch(t, { type: "result", is_error: true, result: "Rate limited", num_turns: 0 });
  const agent2 = makeAgent({ cwd: errored.root, runDir: errored.runDir, env: errored.env, log: (m) => lines.push(m) });
  assert.equal(await agent2("Do it.", { label: "review:US-2" }), null);
  assert.match(lines.at(-1), /^review:US-2: failed after \d+s: Rate limited$/);
});

test("a session still emitting events at its role's limit is stopped with everything in its process group and returns null, and the log says it timed out at its limit", { timeout: 20000 }, async (t) => {
  const s = scratch(t);
  const childPid = join(s.root, "child.pid");
  s.env.FAKE_CLAUDE_HANG = "busy";
  s.env.FAKE_CLAUDE_CHILD = childPid;
  s.env.OMNI_DELIVER_TIMEOUTS = JSON.stringify({ impl: seconds(2.5), stall: seconds(1) });
  const lines = [];
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, env: s.env, log: (m) => lines.push(m) });

  const started = Date.now();
  const out = await agent("Build it.", { label: "impl:US-1", role: "impl", schema: { type: "object", properties: {} } });

  assert.equal(out, null);
  assert.ok(Date.now() - started >= 2500, "it ran to its limit: its events kept the stall timer off");
  assert.match(lines.at(-1), /^impl:US-1: timed out after \d+s, at its limit of 2\.5s$/);
  assert.ok(await gone(s.calls()[0].pid), "the session is stopped");
  assert.ok(await gone(Number(readFileSync(childPid, "utf8"))), "and so is the child it started");
  assert.match(readFileSync(join(s.runDir, "01-impl_US-1.reply.json"), "utf8"), /"type":"assistant"/, "the reply file keeps the events it emitted");
});

test("a session that emits no event for the stall limit is stopped and returns null, and the log says it timed out with no event", { timeout: 20000 }, async (t) => {
  const s = scratch(t);
  s.env.FAKE_CLAUDE_HANG = "silent";
  const lines = [];
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, env: s.env, log: (m) => lines.push(m), timeouts: { stall: seconds(1) } });

  const out = await agent("Review it.", { label: "review:US-1", role: "review", schema: { type: "object", properties: {} } });

  assert.equal(out, null);
  assert.match(lines.at(-1), /^review:US-1: timed out after \d+s, with no event for 1s$/);
  assert.ok(await gone(s.calls()[0].pid));
});

test("a stopped session that ignores SIGTERM gets SIGKILL 10 s later", { timeout: 30000 }, async (t) => {
  const s = scratch(t);
  s.env.FAKE_CLAUDE_HANG = "silent";
  s.env.FAKE_CLAUDE_IGNORE_TERM = "1";
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, env: s.env, timeouts: { verify: seconds(0.5) } });

  const started = Date.now();
  const out = await agent("Check it.", { label: "verify:US-1", role: "verify" });

  assert.equal(out, null);
  assert.ok(Date.now() - started >= 10000, "SIGTERM left it running, so SIGKILL came 10 s after it");
  assert.ok(await gone(s.calls()[0].pid));
});

test("the time limits come from OMNI_DELIVER_TIMEOUTS, then from timeouts in --args, which win", () => {
  assert.deepEqual(timeoutsFrom({ OMNI_DELIVER_TIMEOUTS: '{"impl":120,"stall":30}' }, { impl: 60, stack: 90 }), { impl: 60, stall: 30, stack: 90 });
  assert.deepEqual(timeoutsFrom({}, undefined), {});
  assert.throws(() => timeoutsFrom({ OMNI_DELIVER_TIMEOUTS: "impl=120" }, {}), /^Error: OMNI_DELIVER_TIMEOUTS is not JSON: /);
});

test("the command line becomes the workflow's args, and the feature is required", () => {
  assert.deepEqual(parseArgs(["--feature", "Link a folder", "--implementers", "2", "--story-limit", "4", "--args", '{"models":{"impl":"opus"}}']),
    { feature: "Link a folder", implementers: 2, storyLimit: 4, models: { impl: "opus" } });
  assert.throws(() => parseArgs([]), /pass the feature/);
  assert.throws(() => parseArgs(["--feature", "x", "--bogus"]), /unknown argument --bogus/);
});

test("a schema call whose session answered in text takes the JSON from the text, bare or fenced", async (t) => {
  const bare = scratch(t, { type: "result", is_error: false, result: '{"status":"done","summary":"x"}', num_turns: 7 });
  const agent = makeAgent({ cwd: bare.root, runDir: bare.runDir, env: bare.env });
  assert.deepEqual(await agent("Build it.", { schema: { type: "object", properties: {} }, label: "impl:US-1" }), { status: "done", summary: "x" });

  const fenced = scratch(t, { type: "result", is_error: false, result: '```json\n{"gaps": []}\n```', num_turns: 11 });
  const agent2 = makeAgent({ cwd: fenced.root, runDir: fenced.runDir, env: fenced.env });
  assert.deepEqual(await agent2("Check it.", { schema: { type: "object", properties: {} }, label: "clarity:US-1" }), { gaps: [] });

  const prose = scratch(t, { type: "result", is_error: false, result: "I could not finish.", num_turns: 2 });
  const lines = [];
  const agent3 = makeAgent({ cwd: prose.root, runDir: prose.runDir, env: prose.env, log: (m) => lines.push(m) });
  assert.equal(await agent3("Check it.", { schema: { type: "object", properties: {} }, label: "clarity:US-2" }), null);
  assert.equal(lines.at(-1), "clarity:US-2: the reply holds no structured output and its text is not JSON");
});

test("the phase heading prints when the phase changes, not on every poll that names the same one", () => {
  const lines = [];
  const phase = phaseLogger((m) => lines.push(m));

  for (const title of ["Start", "Unblock", "Unblock", "Unblock", "Review", "Unblock", "Unblock", "Test lane", "Test lane"]) phase(title);

  assert.deepEqual(lines, ["== Start", "== Unblock", "== Review", "== Unblock", "== Test lane"]);
});
