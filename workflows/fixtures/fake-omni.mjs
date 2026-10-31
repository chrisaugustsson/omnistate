// A stand-in for the omni CLI, for workflow tests. Its state (stories, threads,
// posts, a thread's conclusion when it has one, and every call with the lock file seen at that moment) lives in the
// JSON file FAKE_OMNI_STATE names. FAKE_OMNI_FAIL=thread makes thread create fail.
// `checks` answers only when the state holds checks; `story list --pickable` applies
// the board's rule: backlog, not blocked, no planned path shared with a story in
// implementing or review. `story finding` appends to the story's findings and
// `story resolve --finding <i>` resolves the i-th (0-based).
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
    out(state.stories.filter((s) => s.status === "backlog" && !s.blocked && free(s)).map((s) => `${s.key}\n`).join(""));
  } else {
    out(JSON.stringify({ stories: state.stories }));
  }
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
  else if (verb === "edit") s.links = all("--link").map(link);
  else if (verb === "finding") (s.findings ||= []).push({ stage: flag("--stage"), text: flag("--text"), files: all("--file"), resolved: false });
  else if (verb === "resolve") s.findings[Number(flag("--finding"))].resolved = true;
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
