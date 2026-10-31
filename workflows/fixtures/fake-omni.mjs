// A stand-in for the omni CLI, for workflow tests. Its state (stories, threads,
// posts, and every call with the lock file seen at that moment) lives in the
// JSON file FAKE_OMNI_STATE names. FAKE_OMNI_FAIL=thread makes thread create fail.
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
} else if (noun === "post") {
  const body = readFileSync(0, "utf8");
  state.threads.find((t) => t.id === verb).posts.push({ summary: flag("--summary"), body });
  save();
} else if (noun === "story" && verb === "list") {
  save();
  if (argv.includes("--pickable")) {
    out(state.stories.filter((s) => s.status === "backlog" && !s.blocked).map((s) => `${s.key}\n`).join(""));
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
  else if (verb === "decision") s.decisions.push({ question: flag("--question"), choice: flag("--choice"), by: flag("--by") });
  else if (verb === "edit") s.links = all("--link").map(link);
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
