// `make install-agent-tools` against a scratch checkout and a scratch
// CLAUDE_HOME, so the real ~/.claude and this repository are never touched.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, readlinkSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const makefile = fileURLToPath(new URL("../Makefile", import.meta.url));
const helper = fileURLToPath(new URL("./deliver-step.mjs", import.meta.url));
const workflow = fileURLToPath(new URL("./omni-deliver.js", import.meta.url));

function scratch(t) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "install-agent-tools-")));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const repo = join(root, "repo");
  mkdirSync(join(repo, "agent-skills", "omni-grill"), { recursive: true });
  writeFileSync(join(repo, "agent-skills", "omni-grill", "SKILL.md"), "skill\n");
  mkdirSync(join(repo, "workflows"));
  return { root, repo, home: join(root, "claude") };
}

function install({ repo, home }) {
  return execFileSync(
    "make",
    ["-s", "--no-print-directory", "-f", makefile, "-C", repo, "install-agent-tools", `CLAUDE_HOME=${home}`],
    { encoding: "utf8" },
  );
}

test("links omni-grill into the skills directory as an absolute link to the checkout's agent-skills/omni-grill", (t) => {
  const dirs = scratch(t);

  install(dirs);

  const link = join(dirs.home, "skills", "omni-grill");
  assert.equal(lstatSync(link).isSymbolicLink(), true);
  assert.equal(readlinkSync(link), join(dirs.repo, "agent-skills", "omni-grill"));
});

test("replaces a link that points somewhere else with a link to the checkout", (t) => {
  const dirs = scratch(t);
  const elsewhere = join(dirs.root, "elsewhere");
  mkdirSync(elsewhere);
  mkdirSync(join(dirs.home, "skills"), { recursive: true });
  symlinkSync(elsewhere, join(dirs.home, "skills", "omni-grill"));

  install(dirs);

  assert.equal(readlinkSync(join(dirs.home, "skills", "omni-grill")), join(dirs.repo, "agent-skills", "omni-grill"));
  assert.deepEqual(readdirSync(elsewhere), []);
});

test("leaves a real directory in the way untouched and names it in its output", (t) => {
  const dirs = scratch(t);
  const inTheWay = join(dirs.home, "skills", "omni-grill");
  mkdirSync(inTheWay, { recursive: true });
  writeFileSync(join(inTheWay, "SKILL.md"), "hand-written\n");

  const output = install(dirs);

  assert.equal(lstatSync(inTheWay).isDirectory(), true);
  assert.deepEqual(readdirSync(inTheWay), ["SKILL.md"]);
  assert.equal(readFileSync(join(inTheWay, "SKILL.md"), "utf8"), "hand-written\n");
  assert.match(output, new RegExp(inTheWay));
});

test("links each workflow script into the workflows directory", (t) => {
  const dirs = scratch(t);
  writeFileSync(join(dirs.repo, "workflows", "omni-deliver.js"), "export const meta = {};\n");

  install(dirs);

  assert.equal(readlinkSync(join(dirs.home, "workflows", "omni-deliver.js")), join(dirs.repo, "workflows", "omni-deliver.js"));
});

// omni-deliver cannot read HOME: it names its helper with a ~ path for the
// courier's shell to expand, so the link must be where that path points.
// HOME is the scratch one here, and nothing that could carry another
// CLAUDE_HOME reaches make.
test("links the delivery helper where omni-deliver runs it by default, ~/.claude/omni/deliver-step.mjs, and the helper answers through the link", (t) => {
  const dirs = scratch(t);
  copyFileSync(helper, join(dirs.repo, "workflows", "deliver-step.mjs"));
  const home = join(dirs.root, "home");
  const env = { ...process.env, HOME: home };
  for (const name of ["CLAUDE_HOME", "MAKEFLAGS", "MFLAGS", "GNUMAKEFLAGS"]) delete env[name];
  const fallback = readFileSync(workflow, "utf8").match(/^const HELPER = .* : '~\/([^']+)'$/m);
  assert.ok(fallback, "omni-deliver.js names a default helper under ~/");

  execFileSync("make", ["-s", "--no-print-directory", "-f", makefile, "-C", dirs.repo, "install-agent-tools"], { env, encoding: "utf8" });

  const link = join(home, fallback[1]);
  assert.equal(readlinkSync(link), join(dirs.repo, "workflows", "deliver-step.mjs"));
  const answer = JSON.parse(execFileSync(process.execPath, [link, "list"], { cwd: dirs.root, encoding: "utf8" }));
  assert.match(answer.body.error, /^list needs --run <runId> --payload <file> --hash/);
});

test("leaves a real file in the delivery helper's place untouched and names it in its output", (t) => {
  const dirs = scratch(t);
  copyFileSync(helper, join(dirs.repo, "workflows", "deliver-step.mjs"));
  const inTheWay = join(dirs.home, "omni", "deliver-step.mjs");
  mkdirSync(join(dirs.home, "omni"), { recursive: true });
  writeFileSync(inTheWay, "hand-written\n");

  const output = install(dirs);

  assert.equal(lstatSync(inTheWay).isFile(), true);
  assert.equal(readFileSync(inTheWay, "utf8"), "hand-written\n");
  assert.match(output, new RegExp(inTheWay));
});
