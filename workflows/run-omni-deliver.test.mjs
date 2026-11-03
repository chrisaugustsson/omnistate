// The Node runner of omni-deliver: its `agent` against a fake `claude`
// (fixtures/fake-claude.mjs), and its argument parsing.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync, mkdirSync, chmodSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { makeAgent, parseArgs } from "./run-omni-deliver.mjs";

const fakeClaude = fileURLToPath(new URL("./fixtures/fake-claude.mjs", import.meta.url));

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
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const calls = () => readFileSync(logPath, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  return { root, runDir, agentsDir, env, calls };
}

test("a schema call gives claude the prompt on stdin, the schema, model and agent flags, and returns the structured output", async (t) => {
  const s = scratch(t, { type: "result", is_error: false, result: '{"gaps":[]}', structured_output: { gaps: [] }, num_turns: 3, permission_denials: [] });
  const lines = [];
  const agent = makeAgent({ cwd: s.root, runDir: s.runDir, addDirs: ["/tmp/app-stories"], env: s.env, agentsDir: s.agentsDir, log: (m) => lines.push(m) });
  const schema = { type: "object", properties: { gaps: { type: "array" } }, required: ["gaps"] };
  const out = await agent("Check ONE story.", { schema, label: "clarity:US-1", model: "opus", agentType: "verifier" });
  assert.deepEqual(out, { gaps: [] });
  const [call] = s.calls();
  assert.equal(call.prompt, "Check ONE story.");
  assert.equal(call.cwd, s.root);
  const argv = call.argv;
  assert.deepEqual(argv.slice(0, 3), ["-p", "--output-format", "json"]);
  assert.equal(argv[argv.indexOf("--permission-mode") + 1], "auto");
  assert.deepEqual(JSON.parse(argv[argv.indexOf("--json-schema") + 1]), schema);
  assert.equal(argv[argv.indexOf("--model") + 1], "opus");
  assert.ok(!argv.includes("--agent"), "a custom agent would hide the structured output tool");
  const systemFile = argv[argv.indexOf("--append-system-prompt-file") + 1];
  assert.equal(readFileSync(systemFile, "utf8"), "You check work and report.\n", "the agent's body without its front matter");
  assert.equal(argv[argv.indexOf("--disallowedTools") + 1], "Edit,Write,MultiEdit,NotebookEdit", "a read-only agent may not edit");
  assert.equal(argv[argv.indexOf("--add-dir") + 1], "/tmp/app-stories");
  assert.ok(existsSync(join(s.runDir, "01-clarity_US-1.prompt.md")), "the prompt is kept in the run directory");
  assert.ok(existsSync(join(s.runDir, "01-clarity_US-1.reply.json")), "the reply is kept in the run directory");
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
