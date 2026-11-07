#!/usr/bin/env node
// A stand-in for `claude -p` in the runner's tests. It records its arguments,
// the prompt it was given on stdin and its pid to the file FAKE_CLAUDE_LOG,
// then prints the JSON result FAKE_CLAUDE_REPLY holds (a `result` message as
// claude prints it with --output-format json), or exits with FAKE_CLAUDE_EXIT
// when set. With --output-format stream-json it prints one event per line, as
// claude does: an init and an assistant event, then that result.
// FAKE_CLAUDE_HANG=busy prints an assistant event every 50 ms instead and never
// finishes; FAKE_CLAUDE_HANG=silent prints nothing after the init. Both give up
// after a minute. FAKE_CLAUDE_CHILD names a file for the pid of a child it
// starts, which shares its process group and outlives it unless the group is
// stopped; FAKE_CLAUDE_IGNORE_TERM makes it ignore SIGTERM.
import { spawn } from "node:child_process";
import { appendFileSync, writeFileSync } from "node:fs";

if (process.env.FAKE_CLAUDE_IGNORE_TERM) process.on("SIGTERM", () => {});
const argv = process.argv.slice(2);
const streaming = argv[argv.indexOf("--output-format") + 1] === "stream-json";
const event = (e) => process.stdout.write(`${JSON.stringify(e)}\n`);
const assistant = { type: "assistant", message: { role: "assistant", content: [{ type: "text", text: "Working on it." }] }, session_id: "fake" };

let prompt = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (d) => { prompt += d; });
process.stdin.on("end", () => {
  if (process.env.FAKE_CLAUDE_LOG) {
    appendFileSync(process.env.FAKE_CLAUDE_LOG, `${JSON.stringify({ argv, prompt, cwd: process.cwd(), pid: process.pid })}\n`);
  }
  if (process.env.FAKE_CLAUDE_EXIT) {
    process.stderr.write("fake claude: refusing as told\n");
    process.exit(Number(process.env.FAKE_CLAUDE_EXIT));
  }
  if (process.env.FAKE_CLAUDE_CHILD) {
    const child = spawn("sleep", ["60"], { stdio: "ignore" });
    child.unref();
    writeFileSync(process.env.FAKE_CLAUDE_CHILD, String(child.pid));
  }
  if (streaming) event({ type: "system", subtype: "init", cwd: process.cwd(), session_id: "fake", tools: [], model: "fake" });
  if (process.env.FAKE_CLAUDE_HANG) {
    if (process.env.FAKE_CLAUDE_HANG === "busy") setInterval(() => event(assistant), 50);
    setTimeout(() => process.exit(0), 60000);
    return;
  }
  const reply = process.env.FAKE_CLAUDE_REPLY || JSON.stringify({ type: "result", is_error: false, result: "ok", num_turns: 1 });
  if (streaming) event(assistant);
  process.stdout.write(streaming ? `${reply}\n` : reply);
});
