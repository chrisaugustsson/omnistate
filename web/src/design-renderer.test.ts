// @vitest-environment jsdom
// US-058: the renderer's module and .wasm are served at fixed URLs
// (/render/omni_render.js), so a browser can load an older build of them from
// its cache, one without functions that the viewer calls (as
// `_omni_render_visible_boards` was missing when big designs began to load
// board by board, and no board was ever read). `createDesignRenderer` checks
// the module for every function it calls and fails with the names it lacks
// (`missingFunctions`), so the viewer says so instead of failing quietly.
//
// US-31: Escape in play mode leaves play and leaves the design view's selection
// as it was, even before the session has started, while the renderer's own
// Escape (which clears the selection) still listens.
//
// US-32: the viewer asks for animation frames only while a transition plays,
// as long as `play_tick` says 2, and asks for none once it settles.
import { createElement } from "react";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { PlayMode } from "./components/viewer/play";
import { createDesignRenderer, missingFunctions } from "./design/renderer";

/** What the viewer asked of the renderer's module, in order, with the text it passed. */
const calls = vi.hoisted(() => [] as string[]);
/** What `_omni_render_play_tick` says: 2 while a transition plays. */
const tick = vi.hoisted(() => ({ says: 0 }));

vi.mock("/render/omni_render.js", () => {
  const heap = new Uint8Array(4096);
  const text = (pointer: number, length: number) => new TextDecoder().decode(heap.subarray(pointer, pointer + length));
  const called = (name: string) => () => {
    calls.push(name);
    return true;
  };
  const module = new Proxy<Record<string, unknown>>(
    {
      HEAPU8: heap,
      GL: { registerContext: () => 1, makeContextCurrent: () => true },
      UTF8ToString: () => "[]",
      _omni_render_alloc: () => 0,
      _omni_render_free: () => {},
      _omni_render_play_tick: () => tick.says,
      _omni_render_view_open_on: (pointer: number, length: number) => {
        calls.push(`open on "${text(pointer, length)}"`);
        return true;
      },
    },
    { get: (target, name: string) => target[name] ?? (name === "then" ? undefined : called(name)) },
  );
  return { default: async () => module };
});

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

describe("the viewer's renderer", () => {
  beforeEach(() => {
    calls.length = 0;
    vi.stubGlobal("ResizeObserver", class {
      observe() {}
      disconnect() {}
    });
    vi.stubGlobal("requestAnimationFrame", () => 0);
    vi.stubGlobal("cancelAnimationFrame", () => {});
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  async function renderer() {
    const canvas = document.createElement("canvas");
    document.body.append(canvas);
    canvas.getContext = (() => ({})) as unknown as HTMLCanvasElement["getContext"];
    return createDesignRenderer(canvas);
  }

  test("asks the module to open pages on what can be read, before it shows one", async () => {
    (await renderer()).show({}, {});
    const asked = calls.indexOf("_omni_render_view_open_readable");
    expect(asked).toBeGreaterThanOrEqual(0);
    expect(asked).toBeLessThan(calls.indexOf("_omni_render_scene_load"));
  });

  test("tells the module the board a link named before it loads the page", async () => {
    (await renderer()).show({}, {}, "6j8ex84q");
    expect(calls.indexOf('open on "6j8ex84q"')).toBeGreaterThanOrEqual(0);
    expect(calls.indexOf('open on "6j8ex84q"')).toBeLessThan(calls.indexOf("_omni_render_scene_load"));
  });

  test("a selection with reveal moves the view to the node", async () => {
    const shown = await renderer();
    shown.show({}, {});
    shown.select("n0000001", true);
    expect(calls).toContain("_omni_render_view_reveal_selected");
  });

  test("Escape in play mode before the session starts leaves play without clearing the selection", async () => {
    const shown = await renderer();
    shown.show({}, {});
    const onExit = vi.fn();
    render(
      createElement(PlayMode, {
        renderer: shown,
        title: "Checkout screens",
        boards: [{ id: "b0000001", name: "Cart" }],
        flows: [],
        play: "b0000001",
        flow: undefined,
        // The boards are still being read.
        ready: () => new Promise<void>(() => {}),
        onPlay: () => {},
        onExit,
      }),
    );

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(onExit).toHaveBeenCalledExactlyOnceWith("b0000001");
    expect(calls).not.toContain("_omni_render_select_escape");
  });

  test("in play, asks for frames while a transition plays and for none once it settles", async () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => frames.push(callback));
    const shown = await renderer();
    shown.show({}, {});
    shown.play.start("b0000001");
    const run = () => {
      for (const frame of frames.splice(0)) frame(1000);
    };

    tick.says = 2;
    run();
    expect(frames, "a transition plays").toHaveLength(1);

    tick.says = 0;
    run();
    expect(frames, "the transition settled").toHaveLength(0);
  });
});
