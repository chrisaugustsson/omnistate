// US-221: the short title of a behaviour node. A story title matching
// `As a|an <role>, I want [to] <flow>[,] so that <purpose>` reads `<Role>: <flow>`;
// any other title is shown whole. The common form is checked through the
// folders in behaviour-screen.test.tsx; this checks the optional parts.
import { expect, test } from "vitest";
import type { TreeNode } from "@/api/behaviour";
import { shortTitle, shownUnder } from "./components/behaviour/tree";

test("shortTitle shortens `As an` with a want without `to` and no comma before `so that`", () => {
  expect(shortTitle({ title: "As an organiser, I want see the results so that I can rank the players" })).toBe(
    "Organiser: see the results",
  );
});

// US-231: a filter shows the nodes it picks out, every node above one of them,
// and the chosen node with what lies above it, whether or not that one matches.
test("shownUnder keeps the matching nodes, everything above them, and the chosen node with everything above it", () => {
  const node = (id: string, parentId: string | null, defaultAnswers = 0) =>
    ({ id, parent_id: parentId, title: id, status: "answered", default_answers: defaultAnswers, needs_pass: false }) as TreeNode;
  const nodes = [
    node("feature", null),
    node("a", "feature"),
    node("a/1", "a", 2),
    node("a/1/x", "a/1", 1),
    node("a/2", "a"),
    node("b", "feature"),
    node("b/1", "b"),
    node("b/1/y", "b/1"),
    node("b/2", "b"),
  ];

  const { matching, shown } = shownUnder(nodes, "default", "b/1/y");

  expect(matching).toEqual(new Set(["a/1", "a/1/x"]));
  expect(shown).toEqual(new Set(["feature", "a", "a/1", "a/1/x", "b", "b/1", "b/1/y"]));
});
