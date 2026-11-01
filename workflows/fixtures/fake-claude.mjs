#!/usr/bin/env node
// A stand-in for `claude -p` in the runner's tests. It records its arguments
// and the prompt it was given on stdin to the file FAKE_CLAUDE_LOG, then prints
// the JSON result FAKE_CLAUDE_REPLY holds (a `result` message as claude prints
// it with --output-format json), or exits with FAKE_CLAUDE_EXIT when set.
import { appendFileSync } from "node:fs";

let prompt = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (d) => { prompt += d; });
process.stdin.on("end", () => {
  if (process.env.FAKE_CLAUDE_LOG) {
    appendFileSync(process.env.FAKE_CLAUDE_LOG, `${JSON.stringify({ argv: process.argv.slice(2), prompt, cwd: process.cwd() })}\n`);
  }
  if (process.env.FAKE_CLAUDE_EXIT) {
    process.stderr.write("fake claude: refusing as told\n");
    process.exit(Number(process.env.FAKE_CLAUDE_EXIT));
  }
  process.stdout.write(process.env.FAKE_CLAUDE_REPLY || JSON.stringify({ type: "result", is_error: false, result: "ok", num_turns: 1 }));
});
