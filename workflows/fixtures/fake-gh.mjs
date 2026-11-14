// A stand-in for the gh CLI, for workflow tests. Its state (pull requests, every
// call with the folder it was made from, and a failure text or null) lives in the
// JSON file FAKE_GH_STATE names: {prs, calls, fail}. A missing file counts as
// {prs: [], calls: [], fail: null}. A write replaces the file whole.
// Every call is recorded in `calls` as {argv, cwd} before anything else.
// `--version` prints `gh version 0.0.0 (fake)`. While `fail` is a text, every
// other command prints that text on stderr and exits 1.
// `pr list --head <branch> --state open|merged --json <fields>` prints the pull
// requests with that head and state as a JSON array on one line, each with only
// the comma-separated fields asked for.
// `pr create --base <b> --head <h> --title <t> --body-file <file>` adds an open
// pull request with the next number, the file's text as body and the address
// https://example.com/pr/<number>, and prints that address.
// `pr edit <number> --body-file <file>` replaces that pull request's body, and
// fails with `no pull request <number>` for a number the state lacks.
// Flags come in any order after `pr <verb>`.
import { existsSync, readFileSync, renameSync, writeFileSync } from "node:fs";

const argv = process.argv.slice(2);
const statePath = process.env.FAKE_GH_STATE;
const state = { prs: [], calls: [], fail: null, ...(existsSync(statePath) ? JSON.parse(readFileSync(statePath, "utf8")) : {}) };
state.calls.push({ argv, cwd: process.cwd() });

const flag = (name) => {
  const i = argv.indexOf(name);
  return i === -1 ? undefined : argv[i + 1];
};
const save = () => {
  writeFileSync(`${statePath}.${process.pid}.tmp`, JSON.stringify(state, null, 2));
  renameSync(`${statePath}.${process.pid}.tmp`, statePath);
};
const out = (text) => process.stdout.write(text);
const fail = (text, code = 1) => {
  save();
  process.stderr.write(`${text}\n`);
  process.exit(code);
};

const [noun, verb, target] = argv;
if (noun === "--version") {
  save();
  out("gh version 0.0.0 (fake)\n");
} else if (typeof state.fail === "string") {
  fail(state.fail);
} else if (noun === "pr" && verb === "list") {
  const head = flag("--head");
  const wanted = flag("--state");
  const fields = flag("--json")?.split(",");
  const found = state.prs.filter((pr) => (head === undefined || pr.head === head) && (wanted === undefined || pr.state === wanted));
  save();
  out(`${JSON.stringify(found.map((pr) => (fields ? Object.fromEntries(fields.map((f) => [f, pr[f]])) : pr)))}\n`);
} else if (noun === "pr" && verb === "create") {
  const number = Math.max(0, ...state.prs.map((pr) => pr.number)) + 1;
  const url = `https://example.com/pr/${number}`;
  state.prs.push({
    number,
    url,
    head: flag("--head"),
    base: flag("--base"),
    state: "open",
    title: flag("--title"),
    headRefOid: null,
    body: readFileSync(flag("--body-file"), "utf8"),
  });
  save();
  out(`${url}\n`);
} else if (noun === "pr" && verb === "edit") {
  const pr = state.prs.find((x) => String(x.number) === target);
  if (!pr) fail(`no pull request ${target}`);
  pr.body = readFileSync(flag("--body-file"), "utf8");
  save();
} else {
  fail(`fake gh: unknown command ${argv.join(" ")}`, 2);
}
