// A stand-in for the omni CLI, for workflow tests. Its state (stories, threads,
// posts, a thread's conclusion when it has one, and every call with the lock file seen at that moment) lives in the
// JSON file FAKE_OMNI_STATE names. FAKE_OMNI_FAIL=thread makes thread create fail.
// `checks` answers only when the state holds checks; `story list --pickable` applies
// the board's rule: backlog, not blocked, every blocker done or cancelled, no planned
// path shared with a story in implementing or review. `story finding` appends to the story's findings and
// `story resolve --finding <i>` resolves the i-th (0-based). `story add <file>` reads
// the heading, Description, Files and criteria the way the CLI does and numbers
// the story after the highest key; `story tick` sets the criterion's ticked/pending.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";

const statePath = process.env.FAKE_OMNI_STATE;
const state = JSON.parse(readFileSync(statePath, "utf8"));
const argv = process.argv.slice(2);
const lock = existsSync(".omni/run.lock") ? JSON.parse(readFileSync(".omni/run.lock", "utf8")) : null;
state.calls.push({ argv, lock });

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
  if (argv.includes("--pickable")) {
    const busy = state.stories.filter((s) => s.status === "implementing" || s.status === "review").flatMap((s) => s.files_planned || []);
    const free = (s) => !(s.files_planned || []).some((f) => busy.includes(f));
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
  if (verb === "release") s.holder = null;
  else if (verb === "claim") {
    s.holder = { run, stage: flag("--stage") };
    s.status = flag("--stage");
  } else if (verb === "move") s.status = flag("--to");
  else if (verb === "block") s.blocked = true;
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
