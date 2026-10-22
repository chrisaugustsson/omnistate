// US-058: the renderer's module and .wasm are served at fixed URLs
// (/render/omni_render.js), so a browser can load an older build of them from
// its cache, one without functions that the viewer calls (as
// `_omni_render_visible_boards` was missing when big designs began to load
// board by board, and no board was ever read). `createDesignRenderer` checks
// the module for every function it calls and fails with the names it lacks
// (`missingFunctions`), so the viewer says so instead of failing quietly.
import { describe, expect, test } from "vitest";
import { missingFunctions } from "./design/renderer";

describe("the renderer module's functions", () => {
  test("a module with every function the viewer calls lacks none", () => {
    const names = missingFunctions({});
    expect(names).toContain("_omni_render_visible_boards");
    expect(names).toContain("_omni_render_scene_load");
    const module = Object.fromEntries(names.map((name) => [name, () => 0]));
    expect(missingFunctions(module)).toEqual([]);
  });

  test("an older build without visibleBoards is caught by the name it lacks", () => {
    const module: Record<string, unknown> = Object.fromEntries(missingFunctions({}).map((name) => [name, () => 0]));
    delete module._omni_render_visible_boards;
    expect(missingFunctions(module)).toEqual(["_omni_render_visible_boards"]);
  });
});
