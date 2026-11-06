// @vitest-environment jsdom
// US-31: play mode in the design viewer. The toolbar's Play button puts the
// address in play mode (`?play=<board>&flow=<name>`), and `PlayMode`
// (src/components/viewer/play.tsx) fills the window with the prototype: a top
// bar with Exit and the flow select, the stage the renderer draws the player's
// board on, and a bottom bar with Restart, Previous, the position, Next and
// Back. The address follows the board the player is on; Exit and Escape leave
// play on that board.
//
// The player is omni-render's (`play_*`, crates/omni-render/tests/play.rs),
// which jsdom cannot run: the tests give `PlayMode` a fake of the renderer's
// `play` that keeps a board and a history as the player does, and record what
// the page asks of it. The doc page's tests render the app on a memory history
// with a fake of the API, as src/design-viewer.test.tsx does.
//
// What the tests expect, from the story and its decisions:
//
// - Play starts at the first flow's start board with `flow` named, or at the
//   open board without `flow` when the page has no flows.
// - Previous and Next go to the board before and after in the page's board
//   order (pushing the board they leave); Previous is disabled on the first
//   board and Next on the last. The position reads `<name> · <i> of <n>`.
// - Restart starts over at the chosen flow's start, or at the board the session
//   started on when no flow is chosen. Choosing a flow restarts at its start.
//   A `flow` the page lacks shows the select with no flow chosen.
// - The address follows a hotspot click, and an address that moves on its own
//   starts the session over there.
// - A link asks first; Open link opens it with `window.open` in a new tab, or
//   `location.assign`.
// - US-39: the notice reads "This opens <url> outside the prototype.", or "This
//   opens your mail app to write to <address>." for `mailto:`. Cancel and Escape
//   close it and open nothing; Don't show this again is kept under
//   `omni.play.linkNotice` only on Open link, and then the next link opens at
//   once. A link that comes while the notice shows is dropped. A click on
//   nothing says "Nothing happens here. The hotspots are outlined."
// - US-39: Cmd+\ on macOS and Ctrl+\ elsewhere hide both bars (kept in place)
//   and say how to show them again; the same key shows them, and the prototype's
//   key triggers do not see it.
// - A play link to a board the page lacks shows the renderer's reason on the
//   stage, and Exit still leaves.
// - Leaving play names the board the player is on, and only keeps the selected
//   node when it is on that board.
// - A direct play link starts the session only once the page is shown with all
//   its boards read.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { PlayMode, type PlayBoard, type PlayFlow } from "@/components/viewer/play";
import type { DesignRenderer, PlayState } from "@/design/renderer";
import { getRouter } from "./router";
import { answerDesignRead } from "./testing/design-boards";

type FakePlayer = DesignRenderer["play"] & {
  /** What the page asked of the player, in order: `start <board>`, `go <board>`, `restart <board>`, `back`, `stop`. */
  calls: string[];
  /** A click on a hotspot that navigates to `board`. */
  click(board: string): void;
  /** A click on a hotspot that opens `url`. */
  openUrl(url: string, newTab: boolean): void;
  /** A click on nothing that reacts. */
  miss(): void;
};

/** What the fake renderers of the doc page were asked to do. */
const fake = vi.hoisted(() => ({
  instances: [] as {
    shown: unknown[];
    selected: (string | null)[];
    listeners: Set<(address: string | null) => void>;
    player: unknown;
    /** For each session started, the board and how many boards of the page shown were read at that moment. */
    starts: string[];
  }[],
  /** Holds `createDesignRenderer` until it resolves. */
  gate: Promise.resolve() as Promise<void>,
}));

/** A player on the boards `boards()` gives: a board and a history, as omni-render's keeps them. */
function fakePlayer(boards: () => string[]): FakePlayer {
  let state: PlayState | null = null;
  const calls: string[] = [];
  const listeners = new Set<() => void>();
  const notify = () => {
    for (const listener of Array.from(listeners)) listener();
  };
  const now = () => {
    if (!state) throw new Error("no prototype is playing");
    return state;
  };
  const board = (id: string) => {
    if (!boards().includes(id)) throw new Error(`\`${id}\` is not a board of the shown page`);
    return id;
  };
  return {
    calls,
    start(id) {
      calls.push(`start ${id}`);
      state = { board: board(id), history: [], overlays: [], missed: false, url: null };
    },
    stop() {
      calls.push("stop");
      state = null;
    },
    pointer() {
      return false;
    },
    key() {
      return false;
    },
    gamepad() {
      return false;
    },
    back() {
      calls.push("back");
      const shown = now();
      if (shown.history.length === 0) return false;
      state = { ...shown, board: shown.history[shown.history.length - 1], history: shown.history.slice(0, -1) };
      notify();
      return true;
    },
    go(id) {
      calls.push(`go ${id}`);
      const shown = now();
      state = { ...shown, board: board(id), history: [...shown.history, shown.board] };
      notify();
      return true;
    },
    restart(id) {
      calls.push(`restart ${id}`);
      state = { board: board(id), history: [], overlays: [], missed: false, url: null };
      notify();
    },
    state() {
      const shown = now();
      state = { ...shown, missed: false, url: null };
      return shown;
    },
    onChange(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    click(id) {
      const shown = now();
      state = { ...shown, board: board(id), history: [...shown.history, shown.board] };
      act(notify);
    },
    openUrl(url, newTab) {
      state = { ...now(), url: { url, newTab } };
      act(notify);
    },
    miss() {
      state = { ...now(), missed: true };
      act(notify);
    },
  };
}


vi.mock("@/design/renderer", () => ({
  createDesignRenderer: async () => {
    await fake.gate;
    type Shown = { id?: string; children?: Record<string, unknown>[] };
    let current: Shown | undefined;
    const instance = {
      shown: [] as unknown[],
      selected: [] as (string | null)[],
      listeners: new Set<(address: string | null) => void>(),
      player: null as unknown,
      starts: [] as string[],
    };
    fake.instances.push(instance);
    const boards = () => current?.children ?? [];
    const isRead = (board: Record<string, unknown>) => Array.isArray(board.children);
    type Layer = { address: string; name: string; type: string; children: Layer[] };
    const layerOf = (node: Record<string, unknown>): Layer => ({
      address: String(node.id),
      name: String(node.name),
      type: String(node.type),
      children: Array.isArray(node.children) ? (node.children as Record<string, unknown>[]).map(layerOf) : [],
    });
    const find = (layer: Layer, address: string): Layer | null =>
      layer.address === address ? layer : layer.children.reduce<Layer | null>((found, child) => found ?? find(child, address), null);
    const holder = (address: string) => boards().find((board) => find(layerOf(board), address));
    instance.player = fakePlayer(() => boards().map((board) => String(board.id)));
    const player = instance.player as FakePlayer;
    const start = player.start.bind(player);
    player.start = (board, stage) => {
      instance.starts.push(`${board} with ${boards().filter(isRead).length} boards read`);
      start(board, stage);
    };
    return {
      show(page: unknown) {
        instance.shown.push(page);
        current = page as Shown;
      },
      layers() {
        return boards().map((board) => ({ ...layerOf(board), children: [] }));
      },
      fontsStoodIn() {
        return [];
      },
      layersOf(address: string) {
        const board = holder(address);
        if (!board) return null;
        if (!isRead(board)) return String(board.id) === address ? { ...layerOf(board), children: [] } : null;
        return find(layerOf(board), address);
      },
      boardOf(address: string) {
        const board = holder(address);
        return board ? String(board.id) : null;
      },
      select(address: string | null) {
        instance.selected.push(address);
      },
      onSelect(listener: (address: string | null) => void) {
        instance.listeners.add(listener);
        return () => instance.listeners.delete(listener);
      },
      setMode() {},
      inspect() {
        return null;
      },
      boxOf(address: string) {
        return holder(address) ? { x: 0, y: 0, width: 10, height: 10, name: address } : null;
      },
      onView() {
        return () => {};
      },
      visibleBoards() {
        return boards().map((board) => String(board.id));
      },
      addImage() {},
      addFont() {},
      play: instance.player,
      destroy() {},
    };
  },
}));

// PlayMode on its own -------------------------------------------------------------------

const CART = "a0000001";
const RECEIPT = "a0000002";
const DONE = "a0000003";
const BOARDS: PlayBoard[] = [
  { id: CART, name: "Cart" },
  { id: RECEIPT, name: "Receipt" },
  { id: DONE, name: "Done" },
];

function playMode({ play, flow, flows = [] }: { play: string; flow?: string; flows?: PlayFlow[] }) {
  const player = fakePlayer(() => BOARDS.map((board) => board.id));
  const renderer = { play: player } as unknown as DesignRenderer;
  const onPlay = vi.fn();
  const onExit = vi.fn();
  const element = (named: string, chosen: string | undefined) => (
    <PlayMode
      renderer={renderer}
      title="Checkout screens"
      boards={BOARDS}
      flows={flows}
      play={named}
      flow={chosen}
      ready={() => Promise.resolve()}
      onPlay={onPlay}
      onExit={onExit}
    />
  );
  const view = render(element(play, flow));
  return {
    player,
    onPlay,
    onExit,
    /** The address names `named` (and `chosen`) now. */
    address: (named: string, chosen: string | undefined = flow) => view.rerender(element(named, chosen)),
  };
}

const button = (name: string) => screen.getByRole("button", { name: new RegExp(name) });

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the bottom bar", () => {
  test("Next goes to the next board in the page's order, and Previous is disabled on the first board", async () => {
    const { player } = playMode({ play: CART });
    expect(await screen.findByText("Cart · 1 of 3")).toBeTruthy();
    expect(button("Previous")).toHaveProperty("disabled", true);

    fireEvent.click(button("Next"));

    expect(player.calls).toEqual([`start ${CART}`, `go ${RECEIPT}`]);
    expect(await screen.findByText("Receipt · 2 of 3")).toBeTruthy();
  });

  test("Previous goes to the board before in the page's order, and Next is disabled on the last board", async () => {
    const { player } = playMode({ play: DONE });
    expect(await screen.findByText("Done · 3 of 3")).toBeTruthy();
    expect(button("Next")).toHaveProperty("disabled", true);

    fireEvent.click(button("Previous"));

    expect(player.calls).toEqual([`start ${DONE}`, `go ${RECEIPT}`]);
  });

  test("Restart without a flow starts over on the board the session started on", async () => {
    const { player, onPlay } = playMode({ play: RECEIPT });
    await screen.findByText("Receipt · 2 of 3");
    fireEvent.click(button("Next"));
    await screen.findByText("Done · 3 of 3");

    fireEvent.click(button("Restart"));

    expect(player.calls.at(-1)).toBe(`restart ${RECEIPT}`);
    expect(onPlay).toHaveBeenLastCalledWith(RECEIPT, undefined);
    expect(button("Back")).toHaveProperty("disabled", true);
  });

  test("Restart with a flow chosen starts over on the flow's start", async () => {
    const { player, onPlay } = playMode({ play: RECEIPT, flow: "Checkout", flows: [{ name: "Checkout", start: CART }] });
    await screen.findByText("Receipt · 2 of 3");

    fireEvent.click(button("Restart"));

    expect(player.calls.at(-1)).toBe(`restart ${CART}`);
    expect(onPlay).toHaveBeenLastCalledWith(CART, "Checkout");
  });
});

describe("the flow select", () => {
  const flows = [
    { name: "Checkout", start: CART },
    { name: "Refund", start: DONE },
  ];

  test("choosing another flow restarts at its start and names it", async () => {
    const { player, onPlay } = playMode({ play: CART, flow: "Checkout", flows });
    await screen.findByText("Cart · 1 of 3");

    fireEvent.change(screen.getByRole("combobox", { name: /flow/i }), { target: { value: "Refund" } });

    expect(player.calls.at(-1)).toBe(`restart ${DONE}`);
    expect(onPlay).toHaveBeenLastCalledWith(DONE, "Refund");
  });

  test("a flow the page does not have leaves the select with no flow chosen", async () => {
    playMode({ play: RECEIPT, flow: "Gone", flows });
    await screen.findByText("Receipt · 2 of 3");

    expect(screen.getByRole("combobox", { name: /flow/i })).toHaveProperty("value", "");
  });

  test("a page without flows shows no flow select", async () => {
    playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    expect(screen.queryByRole("combobox")).toBeNull();
  });
});

describe("the address", () => {
  test("follows clicks on hotspots, and an address catching up with them does not start the session over", async () => {
    const { player, onPlay, address } = playMode({ play: CART, flow: "Checkout", flows: [{ name: "Checkout", start: CART }] });
    await screen.findByText("Cart · 1 of 3");

    player.click(RECEIPT);
    player.click(DONE);
    expect(onPlay.mock.calls).toEqual([
      [RECEIPT, "Checkout"],
      [DONE, "Checkout"],
    ]);
    // The address names Receipt a moment after the player has gone on to Done.
    address(RECEIPT);

    expect(player.calls).toEqual([`start ${CART}`]);
    expect(screen.getByText("Done · 3 of 3")).toBeTruthy();
  });

  test("an address that moves on its own starts the session over on its board", async () => {
    const { player, address } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    address(DONE);

    expect(player.calls).toEqual([`start ${CART}`, `restart ${DONE}`]);
    expect(await screen.findByText("Done · 3 of 3")).toBeTruthy();
  });
});

describe("a link the prototype opens", () => {
  test("opens in a new tab with window.open", async () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    player.openUrl("https://example.com/terms", true);
    fireEvent.click(await screen.findByRole("button", { name: "Open link" }));

    expect(open).toHaveBeenCalledTimes(1);
    expect(open.mock.calls[0].slice(0, 2)).toEqual(["https://example.com/terms", "_blank"]);
  });

  test("opens in the same tab with location.assign", async () => {
    const assign = vi.fn();
    const open = vi.fn();
    vi.stubGlobal("location", { ...window.location, assign });
    vi.stubGlobal("open", open);
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    player.openUrl("https://example.com/terms", false);
    fireEvent.click(await screen.findByRole("button", { name: "Open link" }));

    expect(assign).toHaveBeenCalledExactlyOnceWith("https://example.com/terms");
    expect(open).not.toHaveBeenCalled();
  });
});

describe("the notice before a link opens", () => {
  const notice = () => screen.findByRole("alertdialog", { name: "Open link?" });

  beforeEach(() => {
    localStorage.clear();
  });

  test("names the link, and Cancel opens nothing", async () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    player.openUrl("https://example.com/terms", true);

    expect((await notice()).textContent).toContain("This opens https://example.com/terms outside the prototype.");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(open).not.toHaveBeenCalled();
  });

  test("a mailto link says it opens the mail app", async () => {
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    player.openUrl("mailto:sales@example.com", true);

    expect((await notice()).textContent).toContain("This opens your mail app to write to sales@example.com.");
  });

  test("Don't show this again with Open link is kept, and the next link opens at once", async () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");
    player.openUrl("https://example.com/terms", true);
    await notice();

    fireEvent.click(screen.getByRole("checkbox", { name: "Don't show this again" }));
    fireEvent.click(screen.getByRole("button", { name: "Open link" }));
    player.openUrl("https://example.com/privacy", true);

    expect(localStorage.getItem("omni.play.linkNotice")).not.toBeNull();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(open.mock.calls.map((call) => call[0])).toEqual(["https://example.com/terms", "https://example.com/privacy"]);
  });

  test("Don't show this again with Cancel is not kept, and the next link asks again", async () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");
    player.openUrl("https://example.com/terms", true);
    await notice();

    fireEvent.click(screen.getByRole("checkbox", { name: "Don't show this again" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    player.openUrl("https://example.com/privacy", true);

    expect(localStorage.getItem("omni.play.linkNotice")).toBeNull();
    expect((await notice()).textContent).toContain("https://example.com/privacy");
    expect(open).not.toHaveBeenCalled();
  });

  test("a link that comes while the notice shows is dropped", async () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");
    player.openUrl("https://example.com/terms", true);
    await notice();

    player.openUrl("https://example.com/privacy", true);
    fireEvent.click(screen.getByRole("button", { name: "Open link" }));

    expect(open.mock.calls.map((call) => call[0])).toEqual(["https://example.com/terms"]);
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  test("Escape closes the notice as Cancel and stays in play", async () => {
    const open = vi.fn();
    vi.stubGlobal("open", open);
    const { player, onExit } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");
    player.openUrl("https://example.com/terms", true);
    await notice();

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(open).not.toHaveBeenCalled();
    expect(onExit).not.toHaveBeenCalled();
  });
});

describe("a click on nothing", () => {
  test("says the hotspots are outlined", async () => {
    const { player } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    player.miss();

    expect((await screen.findByRole("status")).textContent).toBe("Nothing happens here. The hotspots are outlined.");
  });
});

describe("hiding the bars", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("Ctrl+\\ hides both bars in place and says how to show them, and again shows them", async () => {
    vi.spyOn(Navigator.prototype, "platform", "get").mockReturnValue("Win32");
    playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    fireEvent.keyDown(document.body, { key: "\\", ctrlKey: true });

    expect(screen.queryByRole("button", { name: /Exit/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /Restart/ })).toBeNull();
    expect(screen.getByRole("button", { name: /Exit/, hidden: true })).toBeTruthy();
    expect(screen.getByRole("status").textContent).toBe("Press Ctrl+\\ to show the bars again");

    fireEvent.keyDown(document.body, { key: "\\", ctrlKey: true });

    expect(button("Exit")).toBeTruthy();
    expect(button("Restart")).toBeTruthy();
  });

  test("on macOS Cmd+\\ hides the bars and the toast names ⌘", async () => {
    vi.spyOn(Navigator.prototype, "platform", "get").mockReturnValue("MacIntel");
    playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    fireEvent.keyDown(document.body, { key: "\\", metaKey: true });

    expect(screen.queryByRole("button", { name: /Exit/ })).toBeNull();
    expect(screen.getByRole("status").textContent).toBe("Press ⌘\\ to show the bars again");
  });

  test("the prototype's key triggers do not see the key", async () => {
    vi.spyOn(Navigator.prototype, "platform", "get").mockReturnValue("Win32");
    const keys = vi.fn();
    window.addEventListener("keydown", keys);
    playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");

    fireEvent.keyDown(document.body, { key: "\\", ctrlKey: true });
    window.removeEventListener("keydown", keys);

    expect(keys).not.toHaveBeenCalled();
  });
});

describe("leaving play mode", () => {
  test("a play link to a board the page lacks shows the reason on the stage, and Exit leaves on it", async () => {
    const { onExit } = playMode({ play: "zz000009" });

    expect((await screen.findByRole("alert")).textContent).toBe("`zz000009` is not a board of the shown page");
    fireEvent.click(button("Exit"));

    expect(onExit).toHaveBeenCalledExactlyOnceWith("zz000009");
  });

  test("Escape leaves on the board the player is on", async () => {
    const { player, onExit } = playMode({ play: CART });
    await screen.findByText("Cart · 1 of 3");
    player.click(DONE);

    fireEvent.keyDown(document.body, { key: "Escape" });

    expect(onExit).toHaveBeenCalledExactlyOnceWith(DONE);
  });
});

// The doc page --------------------------------------------------------------------------

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const DOC_ID = "0192d7a0-7777-7000-8000-000000000001";
const V1 = "0192d7a0-7777-7000-8000-000000000021";
const DS_V1 = "0192d7a0-7777-7000-8000-000000000022";
const DOC_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/docs/${DOC_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-11-06T10:00:00Z";
const AUTHOR = { run: "run-1", role: "designer", model: "test-model", harness: "test-harness" };

const MOBILE = "pg000001";
const DESKTOP = "pg000002";
const WIDE = "a0000004";
const TALL = "a0000005";
const SUMMARY = "f0000001";

const fixed = (value: number) => ({ type: "fixed", value });
const board = (id: string, name: string, x: number, children: unknown[] = []) => ({
  id,
  type: "board",
  name,
  x,
  y: 0,
  width: fixed(375),
  height: fixed(812),
  layout: { mode: "none" },
  children,
});

// Mobile has the flows Checkout (from Receipt) and Browse (from Cart); Desktop has none.
const PAGES = [
  {
    id: MOBILE,
    type: "page",
    name: "Mobile",
    flows: [
      { name: "Checkout", start: RECEIPT },
      { name: "Browse", start: CART },
    ],
    children: [
      board(CART, "Cart", 0, [
        { id: SUMMARY, type: "frame", name: "Summary", width: fixed(200), height: fixed(100), layout: { mode: "none" } },
      ]),
      board(RECEIPT, "Receipt", 400),
      board(DONE, "Done", 800),
    ],
  },
  {
    id: DESKTOP,
    type: "page",
    name: "Desktop",
    children: [board(WIDE, "Wide", 0), board(TALL, "Tall", 400)],
  },
];

function fakeApi() {
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const doc = {
    id: DOC_ID,
    project_id: PROJECT_ID,
    kind: "design",
    pillar: "ui",
    title: "Checkout screens",
    created_at: AT,
    mainline_version_id: V1,
    drafts: [],
    claims: [],
  };
  const item = { id: V1, doc_id: DOC_ID, draft_id: "0192d7a0-7777-7000-8000-000000000031", removed: false, created_at: AT, agent: AUTHOR };
  const version = {
    ...item,
    design_system_version_id: DS_V1,
    nodes: [],
    tags: '<Page name="Mobile"/>',
    pages: PAGES,
    components: [],
    tokens: {},
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, "http://localhost:3000");
      const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
      const read = answerDesignRead(API, method, url, (id) => (id === DOC_ID ? version : null));
      if (read) return json(read.status, read.body);
      switch (`${method} ${url.pathname}`) {
        case "GET /api/v1/auth/me":
          return json(200, { user: { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" } });
        case "GET /api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
        case `GET ${API}/home`:
          return json(200, { project_id: PROJECT_ID, docs: [doc], threads: [], waiting: { comments: [] } });
        case `GET ${API}/docs/${DOC_ID}`:
          return json(200, { doc });
        case `GET ${API}/docs/${DOC_ID}/versions`:
          return json(200, { doc_id: DOC_ID, versions: [item] });
        case `GET ${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        case `GET ${API}/versions/${V1}`:
          return json(200, { version });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
}

/** A WebSocket that never connects: these tests do not look at live events. */
class QuietWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = QuietWebSocket.CONNECTING;
  onopen = null;
  onmessage = null;
  onclose = null;
  onerror = null;
  send() {}
  close() {
    this.readyState = QuietWebSocket.CLOSED;
  }
}

/**
 * Renders the app at `path` into the document itself, as the browser has it:
 * the app renders <html> and <body> (src/routes/__root.tsx), and play mode is a
 * portal into <body>. Rendered into a <div>, a click sends React's event
 * dispatch round between that root and <body> for good.
 */
function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />, { container: document as unknown as HTMLElement });
  return router;
}

async function findRenderer() {
  return waitFor(() => {
    const latest = fake.instances[fake.instances.length - 1];
    expect(latest, "the page made a renderer").toBeTruthy();
    return latest;
  });
}

/** The toolbar's Play button, once the design can be played. */
async function findPlayButton() {
  return waitFor(() => {
    const play = screen.getByRole("button", { name: /^play$/i });
    expect(play).toHaveProperty("disabled", false);
    return play;
  });
}

async function findPlayDialog() {
  return screen.findByRole("dialog", { name: /play mode/i });
}

function pageButton(name: string) {
  const pages = screen.queryByRole("navigation", { name: /^pages$/i }) ?? screen.getByRole("region", { name: /^pages$/i });
  return within(pages).getByRole("button", { name: new RegExp(`^${name}\\b`) });
}

describe("the doc page", () => {
  beforeEach(() => {
    // A root rendered into a <div> marks the document as listened to, and a
    // root of the document itself would then not listen for clicks at all.
    for (const key of Object.keys(document)) {
      if (key.startsWith("_reactListening")) delete (document as unknown as Record<string, unknown>)[key];
    }
    fake.instances.length = 0;
    fake.gate = Promise.resolve();
    fakeApi();
    vi.stubGlobal("WebSocket", QuietWebSocket);
    window.scrollTo = vi.fn() as unknown as typeof window.scrollTo;
  });

  test("Play plays the page's first flow from its start board, naming the flow", async () => {
    const router = renderAt(DOC_PATH);
    await waitFor(() => expect(router.state.location.search).toEqual({ board: CART }));

    fireEvent.click(await findPlayButton());

    await waitFor(() => expect(router.state.location.search).toMatchObject({ play: RECEIPT, flow: "Checkout" }));
  });

  test("Play on a page without flows plays the open board, naming no flow", async () => {
    const router = renderAt(`${DOC_PATH}?board=${TALL}`);
    await waitFor(() => expect(pageButton("Desktop").getAttribute("aria-current")).toBeTruthy());
    await waitFor(() => expect(router.state.location.search).toEqual({ board: TALL }));

    fireEvent.click(await findPlayButton());

    await waitFor(() => expect(router.state.location.search).toMatchObject({ play: TALL }));
    expect(router.state.location.search).not.toHaveProperty("flow");
  });

  test("Exit returns to the board the player is on, without a selected node of another board, and keeps the selection", async () => {
    const router = renderAt(`${DOC_PATH}?node=${SUMMARY}`);
    const renderer = await findRenderer();
    await waitFor(() => expect(renderer.selected.at(-1)).toBe(SUMMARY));
    fireEvent.click(await findPlayButton());
    const dialog = await findPlayDialog();
    await within(dialog).findByText("Receipt · 2 of 3");

    (renderer.player as FakePlayer).click(DONE);
    await waitFor(() => expect(router.state.location.search).toMatchObject({ play: DONE }));
    fireEvent.click(within(dialog).getByRole("button", { name: /exit/i }));

    await waitFor(() => expect(router.state.location.search).toEqual({ board: DONE }));
    expect(renderer.selected.at(-1)).toBe(SUMMARY);
  });

  test("a play link starts the session once the page is shown with every board read, though the renderer comes last", async () => {
    let release = () => {};
    fake.gate = new Promise<void>((resolve) => (release = resolve));
    renderAt(`${DOC_PATH}?play=${DONE}&flow=Checkout`);
    await waitFor(() => expect(pageButton("Mobile")).toBeTruthy());
    release();
    const renderer = await findRenderer();

    await within(await findPlayDialog()).findByText("Done · 3 of 3");
    expect(renderer.starts).toEqual([`${DONE} with 3 boards read`]);
  });
});
