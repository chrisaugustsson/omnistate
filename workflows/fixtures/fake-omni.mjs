// A stand-in for the omni CLI, for workflow tests. Its state (stories, threads,
// posts, a thread's conclusion when it has one, and every call with the lock file seen at that moment) lives in the
// JSON file FAKE_OMNI_STATE names. FAKE_OMNI_FAIL=thread makes thread create fail.
// While the file FAKE_OMNI_DOWN names exists, `story list` fails with that file's
// text on stderr (the server is down), and the call is recorded with refused: true.
// `checks` answers only when the state holds checks; `story list --pickable` applies
// the board's rule: backlog, not blocked, every blocker done or cancelled, no planned
// file (files_changed when set, else files_planned) shared with a story in implementing, implemented, review, reviewed or testing. `claim` needs the stage's waiting state (else exit 1, not_waiting); `move` to implemented, reviewed, backlog or done clears the holder; `release` and `block` put a working story in its waiting state and clear the holder. `story finding` appends to the story's findings and
// `story resolve --finding <i>` resolves the i-th (0-based). `story add <file>` reads
// the heading, Description, Files and criteria the way the CLI does and numbers
// the story after the highest key; `story tick` sets the criterion's ticked/pending.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

const statePath = process.env.FAKE_OMNI_STATE;
const state = JSON.parse(readFileSync(statePath, "utf8"));
const argv = process.argv.slice(2);
const lock = existsSync(".omni/run.lock") ? JSON.parse(readFileSync(".omni/run.lock", "utf8")) : null;
const down = process.env.FAKE_OMNI_DOWN && existsSync(process.env.FAKE_OMNI_DOWN) && argv[0] === "story" && argv[1] === "list";
state.calls.push(down ? { argv, lock, refused: true } : { argv, lock });

const flag = (name) => {
  const i = argv.indexOf(name);
  return i === -1 ? undefined : argv[i + 1];
};
const all = (name) => argv.flatMap((a, i) => (a === name ? [argv[i + 1]] : []));
const save = () => writeFileSync(statePath, JSON.stringify(state, null, 2));
const out = (text) => process.stdout.write(text);
const story = (key) => {
  const s = state.stories.find((x) => x.key === key);
  if (!s) {
    process.stderr.write(`no story ${key}\n`);
    process.exit(1);
  }
  return s;
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
  out(JSON.stringify({ thread: { id: thread.id, question: thread.title, conclusion: thread.conclusion ?? null, posts: thread.posts } }));
} else if (noun === "checks" && state.checks) {
  save();
  out(JSON.stringify(state.checks));
} else if (noun === "post") {
  const body = readFileSync(0, "utf8");
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
    out(state.stories.filter((s) => s.status === "backlog" && !s.blocked && free(s) && (s.blocked_by || []).every(finished)).map((s) => `${s.key}\n`).join(""));
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
  else if (verb === "edit") { if (all("--link").length) s.links = all("--link").map(link); if (flag("--priority") !== undefined) s.priority = Number(flag("--priority")); }
  else if (verb === "finding") (s.findings ||= []).push({ stage: flag("--stage"), text: flag("--text"), files: all("--file"), resolved: false });
  else if (verb === "resolve") s.findings[Number(flag("--finding"))].resolved = true;
  else if (verb === "tick") {
    const i = Number(flag("--criterion"));
    const c = typeof s.criteria[i] === "string" ? { text: s.criteria[i], human_check: false } : s.criteria[i];
    s.criteria[i] = { ...c, ticked: true, ...(argv.includes("--pending") ? { pending: true } : {}) };
  }
  else if (verb === "files") s.files_changed = argv.slice(3).filter((a, i, rest) => a !== "--run" && a !== "--feature" && rest[i - 1] !== "--run" && rest[i - 1] !== "--feature");
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
