// omni-brownfield.js, run under a fake of the Workflow runtime (agent,
// parallel, pipeline, phase, log), with agents answered from canned results
// by label. It checks the script compiles as the runtime runs it, and how each
// mode fans out: which agents run, and that a run limited to some parts
// builds on an earlier one instead of writing the project docs again.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SOURCE = readFileSync(fileURLToPath(new URL("./omni-brownfield.js", import.meta.url)), "utf8");
const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
const compile = () => new AsyncFunction("args", "agent", "parallel", "pipeline", "phase", "log", SOURCE.replace(/^export const meta = /m, "const meta = "));

const SCOUT = {
  repo: "/repos/shop", remote: "https://github.com/acme/shop.git", branch: "main", shared: [],
  parts: [
    { name: "web", kind: "app", folder: "apps/web", line: "The shop" },
    { name: "api", kind: "service", folder: "services/api", line: "The shop's API" },
  ],
};
const FEATURES = {
  "features web": { description: "The shop app.", features: [{ name: "Cart", line: "Holds what you buy.", screens: true, where: ["apps/web/cart"] }] },
  "features api": { description: "The API.", features: [{ name: "Orders", line: "Takes orders.", screens: false, where: ["services/api/orders"] }] },
};

/** Runs the script with `args`; returns its result, the agents' labels in call order, and their prompts by label. */
async function run(args) {
  const labels = [];
  const prompts = {};
  const agent = async (prompt, opts = {}) => {
    labels.push(opts.label);
    prompts[opts.label] = prompt;
    if (opts.label.startsWith("scout")) return SCOUT;
    if (opts.label.startsWith("features")) return FEATURES[opts.label];
    if (opts.label.startsWith("boards")) return { feature: "Cart", doc: "d1", boards: [{ name: "Cart", id: "b1" }], flows: [], missingTokens: [], notDrawn: [] };
    return { doc: "d", docs: [], gaps: [], part: "p", feature: "f", groups: [], components: [], removed: [], fixed: [], missing: [] };
  };
  const parallel = async (thunks) => Promise.all(thunks.map((thunk) => thunk().catch(() => null)));
  const pipeline = async (items, ...stages) =>
    Promise.all(items.map(async (item, index) => {
      let value = item;
      for (const stage of stages) {
        try {
          value = await stage(value, item, index);
        } catch {
          return null;
        }
      }
      return value;
    }));
  const result = await compile()(args, agent, parallel, pipeline, () => {}, () => {});
  return { result, labels, prompts };
}

test("the map mode scouts each repo, then reads each part's features, and returns the map", async () => {
  const { result, labels } = await run({ mode: "map", repos: ["/repos/shop"] });
  assert.deepEqual(labels, ["scout shop", "features web", "features api"]);
  assert.deepEqual(result.repos[0].parts.map((part) => [part.name, part.features.map((feature) => feature.name)]), [["web", ["Cart"]], ["api", ["Orders"]]]);
});

test("the write mode writes the project docs, the design system, each part, each feature, the boards of a feature with screens, and checks", async () => {
  const { result: map } = await run({ mode: "map", repos: ["/repos/shop"] });
  const { labels, prompts } = await run({ mode: "write", project: "Shop", map });
  assert.deepEqual([...labels].sort(), ["boards Cart", "check", "design system", "how Cart", "how Orders", "part api", "part web", "project docs"]);
  assert.match(prompts["boards Cart"], /export --project "Shop" --doc <doc>/);
  assert.match(prompts["design system"], /\$extensions\.omnistate\.fontFiles/);
});

test("a write limited to some parts writes no project docs and adds to the design system", async () => {
  const { result: map } = await run({ mode: "map", repos: ["/repos/shop"] });
  const { labels, prompts } = await run({ mode: "write", project: "Shop", map, only: ["web"] });
  assert.deepEqual([...labels].sort(), ["boards Cart", "check", "design system", "how Cart", "part web"]);
  assert.match(prompts["design system"], /Keep every token and component it has/);
});
