// fixtures/fake-omni.mjs, the stand-in for the omni CLI: its behaviour tree,
// doc list, link, track export and checks from a file, each run as a child
// process in a temp folder with its state in a JSON file.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const fakeOmni = fileURLToPath(new URL("./fixtures/fake-omni.mjs", import.meta.url));
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\n$/;

// A temp folder to run the fake in, with its state file beside it.
function scratch(t, extra = {}) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "fake-omni-")));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const statePath = join(root, "omni-state.json");
  writeFileSync(statePath, JSON.stringify({ stories: [], threads: [], calls: [], ...extra }));
  const env = { ...process.env, FAKE_OMNI_STATE: statePath, OMNI_RUN: "run-1", OMNI_ROLE: "implementer" };
  for (const name of ["FAKE_OMNI_FAIL", "FAKE_OMNI_DOWN", "FAKE_OMNI_HANG", "FAKE_OMNI_HANG_AFTER"]) delete env[name];
  const state = () => JSON.parse(readFileSync(statePath, "utf8"));
  return {
    root,
    state,
    edit: (change) => {
      const s = state();
      change(s);
      writeFileSync(statePath, JSON.stringify(s));
    },
    // What a call gave: its exit code, standard output and standard error.
    omni: (...argv) => {
      const r = spawnSync(process.execPath, [fakeOmni, ...argv], { cwd: root, env, encoding: "utf8" });
      return { status: r.status, stdout: r.stdout, stderr: r.stderr };
    },
  };
}

const root = { id: "n-root", path: "" };
const rename = { id: "n-rename", path: "rename-a-project" };
const taken = { id: "n-taken", path: "rename-a-project/name-taken" };

function question(id, node, fields = {}) {
  return { id, node_id: node.id, node_path: node.path, question: `Question ${id}?`, recommendation: null, state: "open", answered_by: null, asked_by: { kind: "agent", run: "run-0", role: "clarity" }, answer: null, ...fields };
}

// ---- US-001: the behaviour tree ---------------------------------------------------

test("behaviour list --json prints the state's nodes as {nodes}, and an empty list when the state has no behaviour; behaviour read <node> --json prints the node with its own questions", (t) => {
  const questions = [question("q-1", rename), question("q-2", taken), question("q-3", rename)];
  const dirs = scratch(t, { behaviour: { nodes: [root, rename, taken], questions } });
  const bare = scratch(t);

  const list = dirs.omni("behaviour", "list", "--json", "--track", "board");
  const none = bare.omni("behaviour", "list", "--json", "--track", "board");
  const read = dirs.omni("behaviour", "read", "rename-a-project", "--json", "--track", "board");

  assert.deepEqual(list, { status: 0, stdout: JSON.stringify({ nodes: [root, rename, taken] }), stderr: "" });
  assert.deepEqual(none, { status: 0, stdout: '{"nodes":[]}', stderr: "" });
  assert.deepEqual(read, { status: 0, stdout: JSON.stringify({ node: rename, questions: [questions[0], questions[2]] }), stderr: "" });
  assert.deepEqual(dirs.state().calls.map((c) => c.argv.slice(0, 2).join(" ")), ["behaviour list", "behaviour read"]);
});

test("behaviour ask on a node named by its id, its path, or . for the node whose path is \"\" adds an open question with the recommendation, asked by the agent OMNI_RUN and OMNI_ROLE name, and prints only its ID and a newline", (t) => {
  const dirs = scratch(t, { behaviour: { nodes: [root, rename, taken], questions: [] } });

  const byId = dirs.omni("behaviour", "ask", "n-taken", "Which name wins?", "--recommend", "The older one", "--track", "board");
  const byPath = dirs.omni("behaviour", "ask", "rename-a-project", "Is an empty name allowed?", "--track", "board", "--recommend", "No");
  const byDot = dirs.omni("behaviour", "ask", ".", "Who may rename?", "--recommend", "The owner", "--track", "board");

  for (const r of [byId, byPath, byDot]) {
    assert.deepEqual([r.status, r.stderr], [0, ""]);
    assert.match(r.stdout, uuid);
  }
  const asked_by = { kind: "agent", run: "run-1", role: "implementer" };
  assert.deepEqual(dirs.state().behaviour.questions, [
    { id: byId.stdout.trim(), node_id: "n-taken", node_path: "rename-a-project/name-taken", question: "Which name wins?", recommendation: "The older one", state: "open", answered_by: null, asked_by, answer: null },
    { id: byPath.stdout.trim(), node_id: "n-rename", node_path: "rename-a-project", question: "Is an empty name allowed?", recommendation: "No", state: "open", answered_by: null, asked_by, answer: null },
    { id: byDot.stdout.trim(), node_id: "n-root", node_path: "", question: "Who may rename?", recommendation: "The owner", state: "open", answered_by: null, asked_by, answer: null },
  ]);
  assert.equal(new Set([byId.stdout, byPath.stdout, byDot.stdout]).size, 3);
});

test("behaviour ask and behaviour read on a node the state lacks print not_found: no node <node> in this track on standard error, exit 1, and add nothing", (t) => {
  const questions = [question("q-1", rename)];
  const dirs = scratch(t, { behaviour: { nodes: [root, rename], questions } });
  const bare = scratch(t);

  const ask = dirs.omni("behaviour", "ask", "no-such-node", "Which name wins?", "--recommend", "The older one", "--track", "board");
  const read = dirs.omni("behaviour", "read", "no-such-node", "--json", "--track", "board");
  // The track's name is no node: the flag's value is not taken for one.
  const track = dirs.omni("behaviour", "ask", "board", "Which name wins?", "--track", "rename-a-project");
  const askBare = bare.omni("behaviour", "ask", ".", "Which name wins?", "--track", "board");
  const readBare = bare.omni("behaviour", "read", ".", "--json", "--track", "board");

  assert.deepEqual(ask, { status: 1, stdout: "", stderr: "not_found: no node no-such-node in this track\n" });
  assert.deepEqual(read, { status: 1, stdout: "", stderr: "not_found: no node no-such-node in this track\n" });
  assert.deepEqual(track, { status: 1, stdout: "", stderr: "not_found: no node board in this track\n" });
  assert.deepEqual(askBare, { status: 1, stdout: "", stderr: "not_found: no node . in this track\n" });
  assert.deepEqual(readBare, { status: 1, stdout: "", stderr: "not_found: no node . in this track\n" });
  assert.deepEqual(dirs.state().behaviour, { nodes: [root, rename], questions });
  assert.equal(bare.state().behaviour, undefined);
});

test("behaviour question <id> --json prints {question} with the question's fields, and reads back an answer a person gave that the state file was given", (t) => {
  const dirs = scratch(t, { behaviour: { nodes: [root, rename], questions: [] } });
  const id = dirs.omni("behaviour", "ask", "rename-a-project", "Which name wins?", "--recommend", "The older one", "--track", "board").stdout.trim();
  const asked = { id, node_id: "n-rename", node_path: "rename-a-project", question: "Which name wins?", recommendation: "The older one", state: "open", answered_by: null, asked_by: { kind: "agent", run: "run-1", role: "implementer" }, answer: null };

  const open = dirs.omni("behaviour", "question", id, "--json", "--track", "board");
  dirs.edit((s) => Object.assign(s.behaviour.questions[0], { state: "answered", answered_by: "human", answer: { answer: "The newer one", given_by: "human" } }));
  const answered = dirs.omni("behaviour", "question", id, "--json", "--track", "board");

  assert.deepEqual(open, { status: 0, stdout: JSON.stringify({ question: asked }), stderr: "" });
  assert.deepEqual(answered, { status: 0, stdout: JSON.stringify({ question: { ...asked, state: "answered", answered_by: "human", answer: { answer: "The newer one", given_by: "human" } } }), stderr: "" });
});

// ---- US-002: doc list, link, the track's files and checks from a file --------------

test("doc list --json prints {docs} with the state's docList, and an empty list without one", (t) => {
  const docList = [
    { id: "d-1", kind: "html", pillar: "spec", title: "Spec", track_id: "t-1" },
    { id: "d-2", kind: "design", pillar: "ui", title: "Rename", track_id: "t-1" },
  ];
  const dirs = scratch(t, { docList });
  const bare = scratch(t);

  const list = dirs.omni("doc", "list", "--json", "--track", "board");
  const none = bare.omni("doc", "list", "--json", "--track", "board");

  assert.deepEqual(list, { status: 0, stdout: JSON.stringify({ docs: docList }), stderr: "" });
  assert.deepEqual(none, { status: 0, stdout: '{"docs":[]}', stderr: "" });
});

test("link --json prints the state's link; without one it says on standard error that the folder is not linked, and exits 1", (t) => {
  const link = { path: "/work/app", matched: "/work/app", workspace_id: "w-1", project_id: "p-1" };
  const dirs = scratch(t, { link });
  const bare = scratch(t);

  const linked = dirs.omni("link", "--json");
  const unlinked = bare.omni("link", "--json");

  assert.deepEqual(linked, { status: 0, stdout: JSON.stringify(link), stderr: "" });
  assert.deepEqual(unlinked, { status: 1, stdout: "", stderr: "this folder is not linked to a project: run omni link <project name|id> here, or pass --project\n" });
});

test("export --project p --track f --out <dir> writes for each of the state's boards a non-empty .png and .tags named as the CLI names them, and a manifest.json that lists each board's .tags file", (t) => {
  const boards = [
    { doc: "0190aaaa-d2d2d2d2", board: "b-1", name: "Rename" },
    { doc: "0190aaaa-d2d2d2d2", board: "b-2", name: "Name taken" },
    { doc: "d-3", board: "b-9", name: "settings" },
  ];
  const docList = [{ id: "0190aaaa-d2d2d2d2", kind: "design", pillar: "ui", title: "Rename: screens" }];
  const dirs = scratch(t, { boards, docList });
  const dir = join(dirs.root, "out", "track");

  const r = dirs.omni("export", "--project", "p", "--track", "f", "--out", dir);

  assert.deepEqual([r.status, r.stderr], [0, ""]);
  const files = ["ui/rename-screens-d2d2d2d2/rename-b-1", "ui/rename-screens-d2d2d2d2/name-taken-b-2", "ui/doc-d-3/settings-b-9"];
  for (const file of files) {
    for (const ending of [".png", ".tags"]) {
      const path = join(dir, `${file}${ending}`);
      assert.equal(existsSync(path), true, path);
      assert.ok(readFileSync(path).length > 0, path);
    }
  }
  const listed = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8")).files;
  assert.deepEqual(
    listed.map((f) => [f.path, f.doc_id, f.board_id, f.board]),
    boards.map((b, i) => [`${files[i]}.tags`, b.doc, b.board, b.name]),
  );
});

test("checks --json without checks in the state prints the [checks] table of the folder's .omni.toml as one JSON object; without the file it says so on standard error and exits 1; checks in the state are printed as before", (t) => {
  const dirs = scratch(t);
  const bare = scratch(t);
  const held = scratch(t, { checks: { fast: "make fast" } });
  writeFileSync(join(dirs.root, ".omni.toml"), 'fast = "true"\n');
  writeFileSync(join(held.root, ".omni.toml"), '[checks]\nfast = "true"\n');

  const outside = dirs.omni("checks", "--json", "--track", "board");
  writeFileSync(join(dirs.root, ".omni.toml"), '[checks]\nfast = "true"\n');
  const one = dirs.omni("checks", "--json", "--track", "board");
  writeFileSync(join(dirs.root, ".omni.toml"), '# the project\n[project]\nname = "app"\n\n[checks]\n# quick ones\nfast = "true"\n\nstack_up = "make up NAME=\\"x\\""\n\n[other]\nlint = "make lint"\n');
  const table = dirs.omni("checks", "--json", "--track", "board");
  const missing = bare.omni("checks", "--json", "--track", "board");
  const fromState = held.omni("checks", "--json", "--track", "board");

  assert.deepEqual(outside, { status: 0, stdout: "{}", stderr: "" });
  assert.deepEqual(one, { status: 0, stdout: '{"fast":"true"}', stderr: "" });
  assert.deepEqual(table, { status: 0, stdout: '{"fast":"true","stack_up":"make up NAME=\\"x\\""}', stderr: "" });
  assert.deepEqual(missing, { status: 1, stdout: "", stderr: ".omni.toml not found in this directory or a parent\n" });
  assert.deepEqual(fromState, { status: 0, stdout: '{"fast":"make fast"}', stderr: "" });
});
