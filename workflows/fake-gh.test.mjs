// fixtures/fake-gh.mjs, the stand-in for the gh CLI, run as a child process in
// a temp folder with its state in a JSON file there.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const fakeGh = fileURLToPath(new URL("./fixtures/fake-gh.mjs", import.meta.url));

// A temp folder, the state file in it (written only when a state is given), and
// `gh` to run the fake there.
function scratch(t, state) {
  const dir = realpathSync(mkdtempSync(join(tmpdir(), "fake-gh-")));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const statePath = join(dir, "gh.json");
  if (state) writeFileSync(statePath, JSON.stringify(state));
  const gh = (...argv) => spawnSync(process.execPath, [fakeGh, ...argv], { cwd: dir, env: { ...process.env, FAKE_GH_STATE: statePath }, encoding: "utf8" });
  const read = () => JSON.parse(readFileSync(statePath, "utf8"));
  return { dir, statePath, gh, read };
}

const pr = (number, fields = {}) => ({
  number,
  url: `https://example.com/pr/${number}`,
  head: "h",
  base: "main",
  state: "open",
  title: `Title ${number}`,
  headRefOid: null,
  body: `Body ${number}\n`,
  ...fields,
});

test("every call is recorded, --version prints the fake's version, and a fail text fails every other command", (t) => {
  const { dir, statePath, gh, read } = scratch(t);
  assert.equal(existsSync(statePath), false);

  const version = gh("--version");
  assert.equal(version.status, 0);
  assert.equal(version.stdout, "gh version 0.0.0 (fake)\n");
  assert.equal(version.stderr, "");
  assert.deepEqual(read(), { prs: [], calls: [{ argv: ["--version"], cwd: dir }], fail: null });

  const list = gh("pr", "list", "--head", "h", "--state", "open", "--json", "number");
  assert.equal(list.status, 0);
  assert.equal(list.stdout, "[]\n");

  const unknown = gh("repo", "view");
  assert.equal(unknown.status, 2);
  assert.equal(unknown.stdout, "");
  assert.equal(unknown.stderr, "fake gh: unknown command repo view\n");

  assert.deepEqual(read().calls, [
    { argv: ["--version"], cwd: dir },
    { argv: ["pr", "list", "--head", "h", "--state", "open", "--json", "number"], cwd: dir },
    { argv: ["repo", "view"], cwd: dir },
  ]);

  writeFileSync(statePath, JSON.stringify({ prs: [pr(1)], calls: [], fail: "HTTP 502: Bad Gateway" }));
  writeFileSync(join(dir, "body.md"), "New body\n");
  const failing = [
    ["pr", "list", "--head", "h", "--state", "open", "--json", "number,url"],
    ["pr", "create", "--base", "main", "--head", "h", "--title", "T", "--body-file", join(dir, "body.md")],
    ["pr", "edit", "1", "--body-file", join(dir, "body.md")],
    ["repo", "view"],
  ];
  for (const argv of failing) {
    const failed = gh(...argv);
    assert.equal(failed.status, 1, argv.join(" "));
    assert.equal(failed.stdout, "", argv.join(" "));
    assert.equal(failed.stderr, "HTTP 502: Bad Gateway\n", argv.join(" "));
  }
  const still = gh("--version");
  assert.equal(still.status, 0);
  assert.equal(still.stdout, "gh version 0.0.0 (fake)\n");
  assert.equal(still.stderr, "");

  const after = read();
  assert.deepEqual(after.calls, [...failing, ["--version"]].map((argv) => ({ argv, cwd: dir })));
  assert.deepEqual(after.prs, [pr(1)]);
  assert.equal(after.fail, "HTTP 502: Bad Gateway");
});

test("pr list prints the pull requests with that head and state, each with only the fields asked for", (t) => {
  const prs = [
    pr(1, { head: "h", state: "merged", headRefOid: "aaa111" }),
    pr(2, { head: "h", state: "open" }),
    pr(3, { head: "other", state: "open" }),
    pr(4, { head: "other", state: "merged", headRefOid: "bbb222" }),
    pr(5, { head: "h", state: "open" }),
  ];
  const { gh, read } = scratch(t, { prs, calls: [], fail: null });

  const open = gh("pr", "list", "--head", "h", "--state", "open", "--json", "number,url");
  assert.equal(open.status, 0);
  assert.equal(open.stderr, "");
  assert.equal(open.stdout, '[{"number":2,"url":"https://example.com/pr/2"},{"number":5,"url":"https://example.com/pr/5"}]\n');

  const merged = gh("pr", "list", "--head", "h", "--state", "merged", "--json", "headRefOid");
  assert.equal(merged.status, 0);
  assert.equal(merged.stderr, "");
  assert.equal(merged.stdout, '[{"headRefOid":"aaa111"}]\n');

  const reordered = gh("pr", "list", "--json", "number", "--state", "merged", "--head", "other");
  assert.equal(reordered.status, 0);
  assert.equal(reordered.stdout, '[{"number":4}]\n');

  const none = gh("pr", "list", "--head", "nobody", "--state", "open", "--json", "number,url");
  assert.equal(none.status, 0);
  assert.equal(none.stdout, "[]\n");

  assert.deepEqual(read().prs, prs);
});

test("pr create adds an open pull request with the next number and the file's text as body, and prints its address", (t) => {
  const { dir, gh, read } = scratch(t, { prs: [pr(7)], calls: [], fail: null });
  writeFileSync(join(dir, "body.md"), "## What\n\nA change.\n");

  const created = gh("pr", "create", "--base", "b", "--head", "h", "--title", "t", "--body-file", join(dir, "body.md"));
  assert.equal(created.status, 0);
  assert.equal(created.stderr, "");
  assert.equal(created.stdout, "https://example.com/pr/8\n");
  assert.deepEqual(read().prs, [
    pr(7),
    { number: 8, url: "https://example.com/pr/8", head: "h", base: "b", state: "open", title: "t", headRefOid: null, body: "## What\n\nA change.\n" },
  ]);

  const next = gh("pr", "create", "--body-file", join(dir, "body.md"), "--title", "Second one", "--head", "h2", "--base", "main");
  assert.equal(next.status, 0);
  assert.equal(next.stdout, "https://example.com/pr/9\n");
  assert.deepEqual(read().prs[2], { number: 9, url: "https://example.com/pr/9", head: "h2", base: "main", state: "open", title: "Second one", headRefOid: null, body: "## What\n\nA change.\n" });

  const first = scratch(t);
  writeFileSync(join(first.dir, "body.md"), "First\n");
  const one = first.gh("pr", "create", "--base", "b", "--head", "h", "--title", "t", "--body-file", join(first.dir, "body.md"));
  assert.equal(one.status, 0);
  assert.equal(one.stdout, "https://example.com/pr/1\n");
  assert.deepEqual(first.read().prs, [{ number: 1, url: "https://example.com/pr/1", head: "h", base: "b", state: "open", title: "t", headRefOid: null, body: "First\n" }]);
});

test("pr edit replaces the pull request's body, and fails for a number the state lacks", (t) => {
  const { dir, gh, read } = scratch(t, { prs: [pr(1), pr(2)], calls: [], fail: null });
  writeFileSync(join(dir, "body.md"), "Edited body\n");

  const edited = gh("pr", "edit", "2", "--body-file", join(dir, "body.md"));
  assert.equal(edited.status, 0);
  assert.equal(edited.stderr, "");
  assert.deepEqual(read().prs, [pr(1), pr(2, { body: "Edited body\n" })]);

  const missing = gh("pr", "edit", "3", "--body-file", join(dir, "body.md"));
  assert.equal(missing.status, 1);
  assert.equal(missing.stdout, "");
  assert.equal(missing.stderr, "no pull request 3\n");
  const after = read();
  assert.deepEqual(after.prs, [pr(1), pr(2, { body: "Edited body\n" })]);
  assert.deepEqual(after.calls.at(-1), { argv: ["pr", "edit", "3", "--body-file", join(dir, "body.md")], cwd: dir });
});
