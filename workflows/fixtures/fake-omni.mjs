// A stand-in for the omni CLI, for workflow tests. Its state (stories, threads,
// posts, a thread's conclusion when it has one, and every call with the lock file seen at that moment) lives in the
// JSON file FAKE_OMNI_STATE names. FAKE_OMNI_FAIL=thread makes thread create fail,
// and FAKE_OMNI_FAIL=export makes export fail, writing nothing.
// While the file FAKE_OMNI_DOWN names exists, `story list` fails with that file's
// text on stderr (the server is down), and the call is recorded with refused: true.
// FAKE_OMNI_HANG="<words>" holds a call whose arguments start with those words
// for 30 s before it goes on, as a call stuck on the server would.
// FAKE_OMNI_HANG_AFTER="<words>" holds such a call for 30 s after it saved the
// state: a write that was made, and whose answer never came back.
// FAKE_OMNI_HANG_MS sets how long either holds, for a call that is only slow.
// `checks` prints the state's checks when it holds them; `story list --pickable` applies
// the board's rule: backlog, not blocked, every blocker done or cancelled, no planned
// file (files_changed when set, else files_planned) shared with a story in implementing, implemented, review, reviewed or testing,
// in the board's order: priority, then open (not done or cancelled) dependants, most first, then number. `claim` needs the stage's waiting state (else exit 1, not_waiting); `move` to implemented, reviewed, backlog or done clears the holder; `release` and `block` put a working story in its waiting state and clear the holder. `story finding` appends to the story's findings and
// `story edit` of a story that is done or cancelled is refused (exit 1, not_open), as the board refuses it: no thread can be linked from one.
// `story resolve --finding <i>` resolves the i-th (0-based). `story add <file>` reads
// the heading, Description, Files and criteria the way the CLI does and numbers
// the story after the highest key; `story tick` sets the criterion's ticked/pending.
// `doc read <id>` prints the state's docs[id], and fails for a doc it lacks;
// `thread read` fails for a thread it lacks, with the CLI's not_found.
// `behaviour list`, `read <node>`, `ask <node> <question> [--recommend <text>]` and
// `question <id>` work on the state's behaviour: {nodes: [{id, path}], questions};
// a node is named by its id, its path, or `.` for the one whose path is "", and a
// node or question the state lacks is the CLI's not_found (exit 1). `ask` adds an
// open question asked by the agent OMNI_RUN and OMNI_ROLE name, and prints its ID.
// `doc list` prints the state's docList and `link` its link, which fails as the CLI
// does in a folder that is not linked. The feature's export writes a .png and a .tags
// per entry of the state's boards, named as the CLI names them:
// <out>/ui/<doc title as a slug>-<last 8 characters of the doc ID>/<board name as a
// slug>-<board>, the title taken from docList ("doc" without one), and lists each
// .tags file in <out>/manifest.json as {path, doc_id, board_id, board}. `checks`
// without checks in the state reads the [checks] table of the .omni.toml in the
// current folder, and fails as the CLI does without that file.
// Calls at once each read, change and write the state under a lock file
// beside it, and a write replaces the file whole, as the server serves
// requests at once without losing one.
import { closeSync, existsSync, mkdirSync, openSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { join } from "node:path";

const argv = process.argv.slice(2);
const named = (variable) => process.env[variable] && `${argv.join(" ")} `.startsWith(`${process.env[variable]} `);
const hold = () => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, Number(process.env.FAKE_OMNI_HANG_MS) || 30000);
if (named("FAKE_OMNI_HANG")) hold();
const statePath = process.env.FAKE_OMNI_STATE;
// A lock left by a call that was killed while it held it is taken over
// after 5 s; a call holds it for milliseconds.
const lockPath = `${statePath}.lock`;
for (const until = Date.now() + 30000; ;) {
  try {
    closeSync(openSync(lockPath, "wx"));
    break;
  } catch (e) {
    if (e.code !== "EEXIST") throw e;
    let age = 0;
    try { age = Date.now() - statSync(lockPath).mtimeMs; } catch {}
    if (age > 5000) rmSync(lockPath, { force: true });
    else if (Date.now() > until) throw new Error(`${lockPath} stayed taken for 30 s`);
    else Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5);
  }
}
let locked = true;
const unlock = () => {
  if (!locked) return;
  locked = false;
  rmSync(lockPath, { force: true });
};
process.on("exit", unlock);
const state = JSON.parse(readFileSync(statePath, "utf8"));
const lock = existsSync(".omni/run.lock") ? JSON.parse(readFileSync(".omni/run.lock", "utf8")) : null;
const down = process.env.FAKE_OMNI_DOWN && existsSync(process.env.FAKE_OMNI_DOWN) && argv[0] === "story" && argv[1] === "list";
state.calls.push(down ? { argv, lock, refused: true } : { argv, lock });

const flag = (name) => {
  const i = argv.indexOf(name);
  return i === -1 ? undefined : argv[i + 1];
};
const all = (name) => argv.flatMap((a, i) => (a === name ? [argv[i + 1]] : []));
const save = () => {
  writeFileSync(`${statePath}.${process.pid}.tmp`, JSON.stringify(state, null, 2));
  renameSync(`${statePath}.${process.pid}.tmp`, statePath);
  unlock();
  if (named("FAKE_OMNI_HANG_AFTER")) hold();
};
const out = (text) => process.stdout.write(text);
const story = (key) => {
  const s = state.stories.find((x) => x.key === key);
  if (!s) {
    process.stderr.write(`no story ${key}\n`);
    process.exit(1);
  }
  return s;
};
// The node `given` names: by its id, its path, or `.` for the one whose path is "".
const node = (given) => {
  const n = (state.behaviour?.nodes || []).find((x) => x.id === given || x.path === (given === "." ? "" : given));
  if (!n) {
    save();
    process.stderr.write(`not_found: no node ${given} in this feature\n`);
    process.exit(1);
  }
  return n;
};
// The [checks] table of a .omni.toml: its `name = "command"` lines up to the next
// [header], without blank lines and # comments.
const checksOf = (text) => {
  const checks = {};
  let inside = false;
  for (const line of text.split("\n").map((l) => l.trim())) {
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("[")) {
      inside = line.replace(/\s*#.*$/, "") === "[checks]";
      continue;
    }
    const pair = inside && line.match(/^([\w-]+)\s*=\s*("(?:[^"\\]|\\.)*"|'[^']*')\s*(?:#.*)?$/);
    if (pair) checks[pair[1]] = pair[2].startsWith('"') ? JSON.parse(pair[2]) : pair[2].slice(1, -1);
  }
  return checks;
};
const link = (value) => {
  const [a, b, c] = value.split(":");
  return a === "thread" ? { thread_id: b, label: c ?? "" } : { doc_id: a, node_id: b, label: c ?? "" };
};

const waitingOf = { implementing: "backlog", review: "implemented", testing: "reviewed" };
const waitingFor = { implementing: "backlog", review: "implemented", testing: "reviewed" };

const [noun, verb, target] = argv;
if (noun === "--help") {
  out("omni\n");
} else if (noun === "thread" && verb === "create") {
  if (process.env.FAKE_OMNI_FAIL === "thread") {
    save();
    process.stderr.write("thread create failed\n");
    process.exit(1);
  }
  const id = randomUUID();
  state.threads.push({ id, title: target, feature: flag("--feature"), posts: [] });
  save();
  out(`${id}\n`);
} else if (noun === "thread" && verb === "read") {
  const thread = state.threads.find((t) => t.id === target);
  save();
  if (!thread) {
    process.stderr.write(`not_found: no thread ${target} in this project\n`);
    process.exit(1);
  }
  out(JSON.stringify({ thread: { id: thread.id, question: thread.title, conclusion: thread.conclusion ?? null, posts: thread.posts } }));
} else if (noun === "doc" && verb === "read") {
  save();
  if (state.docs?.[target] === undefined) {
    process.stderr.write(`no doc ${target}\n`);
    process.exit(1);
  }
  out(state.docs[target]);
} else if (noun === "doc" && verb === "list") {
  save();
  out(JSON.stringify({ docs: state.docList ?? [] }));
} else if (noun === "link") {
  save();
  if (!state.link) {
    process.stderr.write("this folder is not linked to a project: run omni link <project name|id> here, or pass --project\n");
    process.exit(1);
  }
  out(JSON.stringify(state.link));
} else if (noun === "export" && process.env.FAKE_OMNI_FAIL === "export") {
  save();
  process.stderr.write("  export failed: the server could not render the boards  \n");
  process.exit(1);
} else if (noun === "export") {
  const slug = (title) => title.split(/[^\p{L}\p{N}]+/u).filter(Boolean).join("-").toLowerCase() || "doc";
  const files = [];
  for (const b of state.boards || []) {
    const title = (state.docList || []).find((d) => d.id === b.doc)?.title ?? "";
    const stem = `ui/${slug(title)}-${b.doc.slice(-8)}`;
    const file = `${stem}/${slug(b.name)}-${b.board}`;
    mkdirSync(join(flag("--out"), stem), { recursive: true });
    writeFileSync(join(flag("--out"), `${file}.png`), Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
    writeFileSync(join(flag("--out"), `${file}.tags`), `<Board name="${b.name}"/>\n`);
    files.push({ path: `${file}.tags`, doc_id: b.doc, pillar: "ui", title, seq: 0, board_id: b.board, board: b.name });
  }
  mkdirSync(flag("--out"), { recursive: true });
  writeFileSync(join(flag("--out"), "manifest.json"), `${JSON.stringify({ files }, null, 2)}\n`);
  save();
} else if (noun === "behaviour" && verb === "list") {
  save();
  out(JSON.stringify({ nodes: state.behaviour?.nodes ?? [] }));
} else if (noun === "behaviour" && verb === "read") {
  const n = node(target);
  save();
  out(JSON.stringify({ node: n, questions: (state.behaviour.questions || []).filter((q) => q.node_id === n.id) }));
} else if (noun === "behaviour" && verb === "ask") {
  const n = node(target);
  const id = randomUUID();
  (state.behaviour.questions ||= []).push({
    id,
    node_id: n.id,
    node_path: n.path,
    question: argv[3],
    recommendation: flag("--recommend") ?? null,
    state: "open",
    answered_by: null,
    asked_by: { kind: "agent", run: process.env.OMNI_RUN ?? null, role: process.env.OMNI_ROLE ?? null },
    answer: null,
  });
  save();
  out(`${id}\n`);
} else if (noun === "behaviour" && verb === "question") {
  const question = (state.behaviour?.questions || []).find((q) => q.id === target);
  save();
  if (!question) {
    process.stderr.write(`not_found: no question ${target} in this feature\n`);
    process.exit(1);
  }
  out(JSON.stringify({ question }));
} else if (noun === "checks" && state.checks) {
  save();
  out(JSON.stringify(state.checks));
} else if (noun === "checks") {
  save();
  if (!existsSync(".omni.toml")) {
    process.stderr.write(".omni.toml not found in this directory or a parent\n");
    process.exit(1);
  }
  out(JSON.stringify(checksOf(readFileSync(".omni.toml", "utf8"))));
} else if (noun === "post") {
  const body = flag("--body-file") ? readFileSync(flag("--body-file"), "utf8") : readFileSync(0, "utf8");
  state.threads.find((t) => t.id === verb).posts.push({ summary: flag("--summary"), body });
  save();
} else if (noun === "story" && verb === "list") {
  save();
  if (down) {
    process.stderr.write(readFileSync(process.env.FAKE_OMNI_DOWN, "utf8") || "Connection refused\n");
    process.exit(1);
  }
  if (argv.includes("--pickable")) {
    const filesOf = (s) => (s.files_changed?.length ? s.files_changed : s.files_planned) || [];
    const busy = state.stories.filter((s) => ["implementing", "implemented", "review", "reviewed", "testing"].includes(s.status)).flatMap(filesOf);
    const free = (s) => !filesOf(s).some((f) => busy.includes(f));
    const finished = (key) => ["done", "cancelled"].includes(state.stories.find((x) => x.key === key)?.status);
    const dependants = (s) => state.stories.filter((x) => !["done", "cancelled"].includes(x.status) && (x.blocked_by || []).includes(s.key)).length;
    const order = (a, b) => (a.priority ?? 3) - (b.priority ?? 3) || dependants(b) - dependants(a) || Number(a.key.slice(3)) - Number(b.key.slice(3));
    out(state.stories.filter((s) => s.status === "backlog" && !s.blocked && free(s) && (s.blocked_by || []).every(finished)).sort(order).map((s) => `${s.key}\n`).join(""));
  } else {
    out(JSON.stringify({ stories: state.stories }));
  }
} else if (noun === "story" && verb === "add") {
  const lines = readFileSync(target, "utf8").split("\n");
  const label = (name) => lines.find((l) => l.startsWith(`**${name}:**`))?.slice(name.length + 5).trim() ?? "";
  const start = lines.findIndex((l) => l.startsWith("**Description:**"));
  const end = lines.findIndex((l, i) => i > start && (l.startsWith("**") || !l.trim()));
  const text = [label("Description"), ...lines.slice(start + 1, end)].join("\n");
  const n = Math.max(0, ...state.stories.map((s) => Number(s.key.slice(3)))) + 1;
  state.stories.push({
    key: `US-${n}`,
    title: lines.find((l) => l.startsWith("### ")).slice(4).trim(),
    text,
    criteria: lines.filter((l) => l.startsWith("- [ ] ")).map((l) => ({ text: l.slice(6), human_check: false })),
    status: "backlog",
    blocked: false,
    holder: null,
    version: 1,
    decisions: [],
    links: all("--link").map(link),
    blocked_by: [],
    files_planned: [...label("Files").matchAll(/`([^`]+)`/g)].map((m) => m[1]),
    priority: Number(flag("--priority") ?? 3),
  });
  save();
  out(`US-${n}\n`);
} else if (noun === "story" && verb === "read") {
  save();
  out(JSON.stringify(story(target)));
} else if (noun === "story") {
  const s = story(target);
  const run = flag("--run");
  if (verb === "release") {
    s.status = waitingOf[s.status] ?? s.status;
    s.holder = null;
  } else if (verb === "claim") {
    const stage = flag("--stage");
    if (s.status !== waitingFor[stage]) {
      save();
      process.stderr.write(`not_waiting: story ${s.key} is ${s.status}, stage ${stage} needs ${waitingFor[stage]}\n`);
      process.exit(1);
    }
    s.holder = { run, stage };
    s.status = stage;
  } else if (verb === "move") {
    s.status = flag("--to");
    if (["implemented", "reviewed", "backlog", "done"].includes(s.status)) s.holder = null;
    else if (s.holder) s.holder = { ...s.holder, stage: s.status };
  } else if (verb === "block") {
    s.blocked = true;
    s.status = waitingOf[s.status] ?? s.status;
    s.holder = null;
  }
  else if (verb === "unblock") s.blocked = false;
  else if (verb === "decision") s.decisions.push({ question: flag("--question"), choice: flag("--choice"), by: flag("--by") });
  else if (verb === "edit") {
    // The board changes nothing of a story that is done or cancelled, its links included.
    if (s.status === "done" || s.status === "cancelled") {
      save();
      process.stderr.write(`not_open: story ${s.key} is ${s.status}\n`);
      process.exit(1);
    }
    if (all("--link").length) s.links = all("--link").map(link);
    if (flag("--priority") !== undefined) s.priority = Number(flag("--priority"));
  }
  else if (verb === "finding") (s.findings ||= []).push({ stage: flag("--stage"), text: flag("--text"), files: all("--file"), resolved: false });
  else if (verb === "resolve") s.findings[Number(flag("--finding"))].resolved = true;
  else if (verb === "tick") {
    const i = Number(flag("--criterion"));
    const c = typeof s.criteria[i] === "string" ? { text: s.criteria[i], human_check: false } : s.criteria[i];
    s.criteria[i] = { ...c, ticked: true, ...(argv.includes("--pending") ? { pending: true } : {}) };
  }
  else if (verb === "files") s.files_changed = argv.slice(3).filter((a, i, rest) => a !== "--run" && a !== "--feature" && a !== "--json" && rest[i - 1] !== "--run" && rest[i - 1] !== "--feature");
  else {
    process.stderr.write(`fake omni: unknown story verb ${verb}\n`);
    process.exit(2);
  }
  s.version += 1;
  save();
  out(JSON.stringify(s));
} else {
  process.stderr.write(`fake omni: unknown command ${argv.join(" ")}\n`);
  process.exit(2);
}
