// Measures each omni-deliver agent type's starting context: for every agent
// file in workflows/agents/, a haiku `claude -p` with the type's tools and its
// body appended to the system prompt, asked for one word, and the tokens of
// its first request (cache write + cache read + input). It spends real
// tokens, so no test runs it: `make measure-agents`.
// A tool the CLI does not have, such as the desktop app's browser
// (mcp__Claude_Browser), is left out by claude, and so is not counted.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const AGENTS_DIR = fileURLToPath(new URL("./agents/", import.meta.url));

// An agent file: frontmatter of `key: value` lines between --- lines, then
// the body, the agent's system prompt.
export function parseAgent(text) {
  const match = text.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
  if (!match) throw new Error("no frontmatter between --- lines");
  const fields = {};
  for (const line of match[1].split("\n")) {
    const at = line.indexOf(": ");
    if (at === -1) throw new Error(`a frontmatter line is not "key: value": ${line}`);
    fields[line.slice(0, at)] = line.slice(at + 2).trim();
  }
  const tools = (fields.tools || "").split(",").map((t) => t.trim()).filter(Boolean);
  return { name: fields.name, description: fields.description, tools, body: match[2].trim() };
}

export function readAgents(dir = AGENTS_DIR) {
  return readdirSync(dir).filter((f) => f.endsWith(".md")).sort().map((file) => ({ file, ...parseAgent(readFileSync(join(dir, file), "utf8")) }));
}

const PROMPT = "Reply with the single word ok.";

function measure(agent, scratch) {
  const bodyFile = join(scratch, `${agent.name}.txt`);
  writeFileSync(bodyFile, `${agent.body}\n`);
  // --tools takes several values, so the prompt goes on stdin.
  const out = execFileSync("claude", ["-p", "--model", "haiku", "--output-format", "json", "--append-system-prompt-file", bodyFile, "--tools", ...agent.tools], {
    input: PROMPT,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  const { usage } = JSON.parse(out);
  return usage.cache_creation_input_tokens + usage.cache_read_input_tokens + usage.input_tokens;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const scratch = mkdtempSync(join(tmpdir(), "measure-agents-"));
  try {
    for (const agent of readAgents()) {
      const tokens = measure(agent, scratch);
      const absent = agent.tools.filter((t) => t.startsWith("mcp__"));
      console.log(`${agent.name.padEnd(14)} ${String(tokens).padStart(7)} tokens  ${agent.tools.join(", ")}${absent.length ? `  (not counted where the CLI lacks them: ${absent.join(", ")})` : ""}`);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}
