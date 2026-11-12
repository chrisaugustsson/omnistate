// US-221: the short title of a behaviour node. A story title matching
// `As a|an <role>, I want [to] <flow>[,] so that <purpose>` reads `<Role>: <flow>`;
// any other title is shown whole. The common form is checked through the
// folders in behaviour-screen.test.tsx; this checks the optional parts.
import { expect, test } from "vitest";
import { shortTitle } from "./components/behaviour/tree";

test("shortTitle shortens `As an` with a want without `to` and no comma before `so that`", () => {
  expect(shortTitle({ title: "As an organiser, I want see the results so that I can rank the players" })).toBe(
    "Organiser: see the results",
  );
});
