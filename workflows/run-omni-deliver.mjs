#!/usr/bin/env node
// Runs workflows/omni-deliver.js under Node, the way its test harness does,
// with a real `agent`: each call is one headless `claude -p` session, given the
// prompt on standard input and asked for JSON (with `--json-schema` when the
// workflow passes a schema). The workflow runtime of the desktop app cannot run
// the script, since it drives git and omni through node:child_process.
//
//   node workflows/run-omni-deliver.mjs --feature "<name>" [--implementers 3]
//        [--reviewers 3] [--story-limit n] [--model <alias>] [--args '<json>']
//
// Run it from the main worktree with a clean tree. Progress goes to standard
// error, the workflow's result to standard output as JSON, and every agent's
// prompt and reply to .omni/runs/<stamp>/ for reading afterwards.
//
// Environment: OMNI_DELIVER_CLAUDE (the claude binary, `claude` by default),
// OMNI_DELIVER_PERMISSION_MODE (`auto` by default), OMNI_DELIVER_MAX_TURNS
// (unset by default: no cap).

import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const scriptPath = fileURLToPath(new URL("./omni-deliver.js", import.meta.url));

/** Turns `--feature x --implementers 2 --args '{"k":1}'` into the workflow's args. */
export function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const word = argv[i];
    const value = argv[i + 1];
    switch (word) {
      case "--feature": args.feature = value; i++; break;
      case "--implementers": args.implementers = Number(value); i++; break;
      case "--reviewers": args.reviewers = Number(value); i++; break;
      case "--story-limit": args.storyLimit = Number(value); i++; break;
      case "--model": args.model = value; i++; break;
      case "--args": Object.assign(args, JSON.parse(value)); i++; break;
      default: throw new Error(`unknown argument ${word}; see the header of workflows/run-omni-deliver.mjs`);
    }
  }
  if (!args.feature) throw new Error('pass the feature: --feature "<name>"');
  return args;
}

const pad = (n) => String(n).padStart(2, "0");
const stamp = (d = new Date()) =>
  `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
const clock = () => new Date().toISOString().slice(11, 19);

/**
 * The `agent` the workflow calls. Each call runs `claude -p` in `cwd` with the
 * prompt on stdin and returns the structured output (with a schema) or the
 * final text; a failed or errored session returns null, as the workflow
 * runtime's agent() does, and the reason is logged.
 */
export function makeAgent({ cwd, runDir, addDirs = [], env = process.env, log = () => {}, agentsDir = join(env.HOME || "", ".claude", "agents") }) {
  let count = 0;
  const bin = env.OMNI_DELIVER_CLAUDE || "claude";
  const permissionMode = env.OMNI_DELIVER_PERMISSION_MODE || "auto";
  const roles = new Map();
  // A custom agent (`--agent`) hides the structured output tool, so a session
  // started that way answers in text and ignores --json-schema. The agent's
  // instructions go in through the system prompt instead, and its tool list
  // decides whether the session may edit files.
  const roleOf = (type) => {
    if (!roles.has(type)) {
      const text = readFileSync(join(agentsDir, `${type}.md`), "utf8");
      const parts = text.split(/^---\s*$/m);
      const front = parts.length >= 3 ? parts[1] : "";
      const body = (parts.length >= 3 ? parts.slice(2).join("---") : text).trim();
      const tools = (front.match(/^tools:\s*(.+)$/m)?.[1] || "").split(",").map((t) => t.trim()).filter(Boolean);
      const file = join(runDir || cwd, `${type}.system.md`);
      writeFileSync(file, `${body}\n`);
      const editing = tools.length === 0 || tools.some((t) => ["Edit", "Write", "MultiEdit", "NotebookEdit"].includes(t));
      roles.set(type, { file, disallowed: editing ? [] : ["Edit", "Write", "MultiEdit", "NotebookEdit"] });
    }
    return roles.get(type);
  };
  return async function agent(prompt, opts = {}) {
    const n = ++count;
    const label = opts.label || `agent-${n}`;
    const file = `${pad(n)}-${label.replace(/[^a-zA-Z0-9._-]+/g, "_")}`;
    const argv = ["-p", "--output-format", "json", "--permission-mode", permissionMode];
    if (opts.schema) argv.push("--json-schema", JSON.stringify(opts.schema));
    if (opts.model) argv.push("--model", opts.model);
    if (opts.agentType) {
      const role = roleOf(opts.agentType);
      argv.push("--append-system-prompt-file", role.file);
      if (role.disallowed.length) argv.push("--disallowedTools", role.disallowed.join(","));
    }
    if (env.OMNI_DELIVER_MAX_TURNS) argv.push("--max-turns", env.OMNI_DELIVER_MAX_TURNS);
    for (const dir of addDirs) argv.push("--add-dir", dir);
    if (runDir) writeFileSync(join(runDir, `${file}.prompt.md`), prompt);
    log(`${label}: starting (${opts.model || "default model"}${opts.agentType ? `, ${opts.agentType}` : ""})`);
    const started = Date.now();
    const { code, stdout, stderr } = await run(bin, argv, { cwd, env, input: prompt });
    const seconds = Math.round((Date.now() - started) / 1000);
    if (runDir) writeFileSync(join(runDir, `${file}.reply.json`), stdout || stderr);
    let reply = null;
    try { reply = JSON.parse(stdout); } catch { reply = null; }
    if (code !== 0 || !reply || reply.is_error) {
      const why = reply?.result || stderr.trim().split("\n").pop() || `exit ${code}`;
      log(`${label}: failed after ${seconds}s: ${String(why).slice(0, 300)}`);
      return null;
    }
    const denials = reply.permission_denials?.length || 0;
    log(`${label}: done in ${seconds}s, ${reply.num_turns ?? "?"} turns${denials ? `, ${denials} permission denials` : ""}`);
    if (opts.schema) {
      if (reply.structured_output != null) return reply.structured_output;
      // A session sometimes answers with the JSON as text instead of calling
      // the structured output tool; that answer is as good.
      const parsed = parseJsonText(reply.result);
      if (parsed === null) log(`${label}: the reply holds no structured output and its text is not JSON`);
      return parsed;
    }
    return typeof reply.result === "string" ? reply.result : null;
  };
}

/** The JSON object in `text`, bare or inside a ```json fence; null when there is none. */
export function parseJsonText(text) {
  if (typeof text !== "string") return null;
  let body = text.trim();
  const fence = body.match(/^```(?:json)?\s*([\s\S]*?)\s*```$/);
  if (fence) body = fence[1];
  try {
    const value = JSON.parse(body);
    return value && typeof value === "object" ? value : null;
  } catch {
    return null;
  }
}

function run(bin, argv, { cwd, env, input }) {
  return new Promise((resolve) => {
    const child = spawn(bin, argv, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => { stdout += d; });
    child.stderr.on("data", (d) => { stderr += d; });
    child.on("error", (e) => resolve({ code: -1, stdout, stderr: `${stderr}\n${e.message}` }));
    child.on("close", (code) => resolve({ code, stdout, stderr }));
    child.stdin.end(input);
  });
}

/** Loads the workflow script as the test harness does: a function of the runtime hooks. */
export function loadWorkflow(path = scriptPath) {
  const source = readFileSync(path, "utf8").replace("export const meta", "const meta");
  const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
  return new AsyncFunction("args", "agent", "phase", "parallel", "log", source);
}

/** The `== <phase>` heading for the log: the loops name their phase on every poll, so a heading prints only when it changes. */
export function phaseLogger(log) {
  let heading = null;
  return (title) => {
    if (title === heading) return;
    heading = title;
    log(`== ${title}`);
  };
}

export async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  const cwd = process.cwd();
  const runDir = join(cwd, ".omni", "runs", stamp());
  mkdirSync(runDir, { recursive: true });
  const log = (message) => process.stderr.write(`${clock()} ${message}\n`);
  const phase = phaseLogger(log);
  const parallel = (thunks) => Promise.all(thunks.map((t) => t().catch((e) => { log(`a parallel task failed: ${e.message}`); return null; })));
  const storiesDir = join(cwd, "..", `${cwd.split("/").pop()}-stories`);
  const agent = makeAgent({ cwd, runDir, addDirs: [storiesDir], log });
  log(`omni-deliver on "${args.feature}"; agent transcripts in ${runDir}`);
  const workflow = loadWorkflow();
  const result = await workflow(args, agent, phase, parallel, log);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  return result;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  main().catch((e) => {
    process.stderr.write(`omni-deliver: ${e.message}\n`);
    process.exit(1);
  });
}
