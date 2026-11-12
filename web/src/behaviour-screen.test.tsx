// @vitest-environment jsdom
// US-221: a feature's Behaviour screen, .../features/<feature>/behaviour. It
// reads GET .../features/<feature>/behaviour and lists the tree as folders: the
// feature first, then its stories, closed, by their short titles. An empty list
// is a feature with no tree; any failed request, 404 included, is a failed load.
// The app is rendered with its own router on a memory history and the
// browser's fetch is replaced by a fake of the API.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { testTree } from "./testing/behaviour-test-tree";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const FEATURE_ID = "0192d7a0-4444-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const BEHAVIOUR_PATH = `${PROJECT_PATH}/features/${FEATURE_ID}/behaviour`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-11-12T09:00:00Z";

const ROOT = "0192d7a0-8888-7000-8000-000000000001";
const SIGN_UP_STORY = "0192d7a0-8888-7000-8000-000000000002";
const SIGNS_UP = "0192d7a0-8888-7000-8000-000000000003";
const CONFIRMS = "0192d7a0-8888-7000-8000-000000000004";
const SKIPS = "0192d7a0-8888-7000-8000-000000000005";
const PROGRAM_STORY = "0192d7a0-8888-7000-8000-000000000006";

/** A node of the list GET .../behaviour sends. */
function node(id: string, parentId: string | null, title: string, fields: { status?: string; position?: number } = {}) {
  return {
    id,
    parent_id: parentId,
    path: "",
    slug: null,
    position: fields.position ?? 0,
    level: 0,
    title,
    summary: "",
    status: fields.status ?? "answered",
    version: 1,
    open_questions: 0,
    default_answers: 0,
    needs_pass: false,
    same_as: null,
    handoff: null,
  };
}

/**
 * The feature, a story with a situation that has one below it, a situation
 * after it that is out of scope, and a second story with nothing under it.
 */
const TREE = [
  node(ROOT, null, "Disc golf putting trainer"),
  node(SIGN_UP_STORY, ROOT, "As a player, I want to create an account and get set up, so that my practice is saved"),
  node(SIGNS_UP, SIGN_UP_STORY, "Player signs up with an email"),
  node(CONFIRMS, SIGNS_UP, "Player confirms the email"),
  node(SKIPS, SIGN_UP_STORY, "Player skips the setup", { status: "out_of_scope", position: 1 }),
  node(PROGRAM_STORY, ROOT, "As a player, I want to pick a training program, so that I know what to practise", { position: 1 }),
];

type Behaviour = { status: number; body: unknown };

/** A feature of the project's home data. */
function feature(id: string, name: string) {
  return {
    feature: { id, project_id: PROJECT_ID, name, description: "", created_at: AT },
    docs: 0,
    waiting: 0,
    stories: { total: 0, open: 0, done: 0 },
    last_activity: null,
  };
}

/** An open question on `nodeId` of the list GET .../behaviour/questions?state=open sends. */
function openQuestion(id: string, nodeId: string, askedBy: Record<string, unknown>) {
  return { id, node_id: nodeId, node_path: "", question: "Which putt counts?", state: "open", asked_by: askedBy, answers: [], created_at: AT, version: 1 };
}

const BY_AGENT = { kind: "agent", run: "r1", role: "grill", model: "m", harness: "h" };
const BY_PERSON = { kind: "human", user_id: USER.id };

/**
 * A fake of the API whose behaviour list answers with `behaviour` and whose
 * home data holds `features`, or never arrives when `features` is "pending".
 * The open questions are `questions`; without them that request fails.
 */
function fakeApi(behaviour: Behaviour, features: unknown[] | "pending" = [], questions?: unknown[]) {
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Trainer", created_at: AT }] });
        case `${API}/home`:
          if (features === "pending") return new Promise<Response>(() => {});
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            features,
            docs: [],
            threads: [],
            waiting: { comments: [] },
          });
        case `${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        case `${API}/features/${FEATURE_ID}/behaviour`:
          return json(behaviour.status, behaviour.body);
        case `${API}/features/${FEATURE_ID}/behaviour/questions`:
          if (!questions) return json(404, { error: "not_found" });
          return json(200, { questions });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
}

/** A WebSocket that never opens: this story reads the tree once. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(readonly url: string) {
    super();
  }

  send() {
    throw new Error("InvalidStateError: the socket is not open");
  }

  close() {
    this.readyState = FakeWebSocket.CLOSED;
  }
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The folders, once the tree is loaded. */
async function folders() {
  return screen.findByRole("complementary", { name: "Tree as folders" });
}

/** The folders' rows, top to bottom, each as its shown text. */
function rowTexts(aside: HTMLElement) {
  return rows(aside).map((row) => row.textContent);
}

function rows(aside: HTMLElement) {
  return Array.from((aside.lastElementChild as HTMLElement).children) as HTMLElement[];
}

function row(aside: HTMLElement, text: string) {
  const found = rows(aside).find((candidate) => candidate.textContent === text);
  expect(found, `a row "${text}"`).toBeTruthy();
  return found as HTMLElement;
}

const chevron = (aside: HTMLElement, text: string) => within(row(aside, text)).queryByRole("button");

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the Behaviour screen's folders", () => {
  test("list the feature, then its stories closed by their short titles, under the count of stories and nodes", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    expect(screen.getByRole("heading", { level: 1, name: "Behaviour" })).toBeTruthy();
    expect(within(aside).getByText("2 stories · 6 nodes")).toBeTruthy();
    expect(rowTexts(aside)).toEqual([
      "Disc golf putting trainer",
      "Player: create an account and get set up",
      "Player: pick a training program",
    ]);
    expect(within(row(aside, "Disc golf putting trainer")).queryByRole("button")).toBeNull();
  });

  test("a story's chevron shows its children in tree order and hides them again; a node with nothing under it has no chevron", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    expect(chevron(aside, "Player: pick a training program")).toBeNull();

    fireEvent.click(chevron(aside, "Player: create an account and get set up") as HTMLElement);
    expect(rowTexts(aside)).toEqual([
      "Disc golf putting trainer",
      "Player: create an account and get set up",
      "Player signs up with an email",
      "Player skips the setup",
      "Player: pick a training program",
    ]);
    expect(chevron(aside, "Player skips the setup")).toBeNull();

    fireEvent.click(chevron(aside, "Player: create an account and get set up") as HTMLElement);
    expect(rowTexts(aside)).toEqual([
      "Disc golf putting trainer",
      "Player: create an account and get set up",
      "Player: pick a training program",
    ]);
  });

  test("a row for an out_of_scope node has its title struck through and other rows do not", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    fireEvent.click(chevron(aside, "Player: create an account and get set up") as HTMLElement);
    expect(within(aside).getByText("Player skips the setup").className).toMatch(/\bline-through\b/);
    expect(within(aside).getByText("Player signs up with an email").className).not.toMatch(/\bline-through\b/);
  });
});

// US-225: a row's badge counts the questions that wait for an answer: those an
// agent asked. A question a person asked is a remark and never counts (C2). A
// closed row with something under it adds up its own and everything under it;
// an open row counts only its own.
describe("the Behaviour screen's waiting badges in the folders", () => {
  /** The row whose title is `title`, whatever marks follow the title. */
  function rowTitled(aside: HTMLElement, title: string) {
    const found = rows(aside).find((candidate) => within(candidate).queryByText(title) !== null);
    expect(found, `a row "${title}"`).toBeTruthy();
    return found as HTMLElement;
  }

  const badge = (aside: HTMLElement, title: string) =>
    rowTitled(aside, title).querySelector<HTMLElement>('[data-mark="waiting"]');

  test("a closed row adds up the questions agents asked on it and under it, an open row counts its own, and remarks never count", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } }, [], [
      openQuestion("q1", SIGNS_UP, BY_AGENT),
      openQuestion("q2", CONFIRMS, BY_AGENT),
      openQuestion("q3", CONFIRMS, BY_PERSON),
      openQuestion("q4", SKIPS, BY_PERSON),
    ]);
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    const story = "Player: create an account and get set up";

    await waitFor(() => expect(badge(aside, story)?.textContent).toBe("2"));
    expect(badge(aside, story)!.title).toBe("Open questions here and inside");

    fireEvent.click(within(rowTitled(aside, story)).getByRole("button"));
    expect(badge(aside, story)).toBeNull();
    expect(badge(aside, "Player signs up with an email")?.textContent).toBe("2");
    expect(badge(aside, "Player signs up with an email")!.title).toBe("Open questions here and inside");
    expect(badge(aside, "Player skips the setup")).toBeNull();

    fireEvent.click(within(rowTitled(aside, "Player signs up with an email")).getByRole("button"));
    expect(badge(aside, "Player signs up with an email")?.textContent).toBe("1");
    expect(badge(aside, "Player confirms the email")?.textContent).toBe("1");
  });
});

describe("the Behaviour screen with no tree or a failed load", () => {
  test("an empty list says no behaviour has been written, in place of the folders", async () => {
    fakeApi({ status: 200, body: { nodes: [] } });
    renderAt(BEHAVIOUR_PATH);
    expect(await screen.findByText("No behaviour has been written for this feature yet.")).toBeTruthy();
    expect(screen.queryByRole("complementary", { name: "Tree as folders" })).toBeNull();
  });

  test("a tree holding only the feature is drawn as a tree, not as the empty state", async () => {
    fakeApi({ status: 200, body: { nodes: [node(ROOT, null, "Disc golf putting trainer")] } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    expect(within(aside).getByText("0 stories · 1 node")).toBeTruthy();
    expect(rowTexts(aside)).toEqual(["Disc golf putting trainer"]);
    expect(screen.queryByText("No behaviour has been written for this feature yet.")).toBeNull();
  });

  test("a 404 for the list is a failed load: the alert shows and neither folders nor the empty state do", async () => {
    fakeApi({ status: 404, body: { error: "not_found" } });
    renderAt(BEHAVIOUR_PATH);
    expect(await screen.findByText("The behaviour tree could not be loaded. Try again in a moment.")).toBeTruthy();
    expect(screen.queryByRole("complementary", { name: "Tree as folders" })).toBeNull();
    expect(screen.queryByText("No behaviour has been written for this feature yet.")).toBeNull();
  });
});

// US-222: the screen takes the whole window. The app's sidebar is left out, and
// the top bar links back to the feature by the name the project's home data
// gives it, never by the tree's root title.
describe("the Behaviour screen's frame and top bar", () => {
  test("leaves out the app's sidebar", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    expect(screen.queryByRole("navigation", { name: "Project" })).toBeNull();
  });

  test("links back to the feature page by the feature's name while the first row shows the root title", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } }, [feature(FEATURE_ID, "Putting basics")]);
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    const back = await screen.findByRole("link", { name: "‹ Putting basics" });
    expect(back.getAttribute("href")).toBe(`${PROJECT_PATH}/features/${FEATURE_ID}`);
    expect(rowTexts(aside)[0]).toBe("Disc golf putting trainer");
  });

  test("before the home data arrives the back link reads only the arrow, not the root title", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } }, "pending");
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const back = screen.getByRole("link", { name: "‹" });
    expect(back.textContent).toBe("‹ ");
    expect(back.getAttribute("href")).toBe(`${PROJECT_PATH}/features/${FEATURE_ID}`);
  });
});

// US-226: one chosen node for the folders, the graph and the details. Here the
// graph is drawn in a graph area of 748 x 844 and the view is read back from
// the drawing's `translate(Xpx, Ypx) scale(K)`. Ids in the PRD's test tree are
// folder paths; its root is "feature".
describe("choosing a node in the Behaviour screen", () => {
  const AREA = { w: 748, h: 844 };
  const SIGNS_IN = "player-sets-up-account/player-signs-in-to-existing-account";
  const CANNOT_REMEMBER = `${SIGNS_IN}/player-cannot-remember-sign-in`;
  const SEARCHES = "player-adds-friend/player-searches-for-player";
  type View = { x: number; y: number; k: number };

  function currentView(): View {
    const drawing = document.querySelector<HTMLElement>('main [style*="scale("]')!;
    const [, x, y, k] = /translate\((-?[\d.e-]+)px, (-?[\d.e-]+)px\) scale\(([\d.e-]+)\)/.exec(drawing.style.transform)!;
    return { x: Number(x), y: Number(y), k: Number(k) };
  }

  /** Waits until the graph shows a view and returns it. */
  async function openingView() {
    await waitFor(() => expect(document.querySelector('main [style*="scale("]')).not.toBeNull());
    return currentView();
  }

  /** Waits for a move to land: two samples 150ms apart show the same view. */
  async function landed() {
    let last: View | undefined;
    await waitFor(
      () => {
        const now = currentView();
        const still = last !== undefined && now.x === last.x && now.y === last.y && now.k === last.k;
        last = now;
        expect(still).toBe(true);
      },
      { interval: 150, timeout: 2000 },
    );
    return currentView();
  }

  const boxOf = (id: string) => document.querySelector<HTMLElement>(`main [data-node="${id}"]`)!;
  const rowOf = (id: string) => document.querySelector<HTMLElement>(`aside [data-row="${id}"]`);
  const detailsTitle = () => screen.getByRole("region", { name: "Details" }).querySelector("h2")?.textContent ?? null;
  const graphArea = () => screen.getByRole("main", { name: "Tree as a graph" }).firstElementChild as HTMLElement;

  /** Presses `element` without dragging. */
  function press(element: HTMLElement) {
    fireEvent.pointerDown(element, { pointerId: 1, button: 0, clientX: 100, clientY: 100 });
    fireEvent.pointerUp(element, { pointerId: 1, button: 0, clientX: 100, clientY: 100 });
  }

  /** Where a box lies in the drawing, from its translate, width and height. */
  function rectOf(element: HTMLElement) {
    const [, x, y] = /translate\((-?[\d.]+)px, (-?[\d.]+)px\)/.exec(element.style.transform)!;
    return { x: Number(x), y: Number(y), w: parseFloat(element.style.width), h: parseFloat(element.style.height) };
  }

  beforeEach(() => {
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(AREA.w);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(AREA.h);
    const captured = new Set<number>();
    Element.prototype.setPointerCapture = (id: number) => void captured.add(id);
    Element.prototype.hasPointerCapture = (id: number) => captured.has(id);
  });

  afterEach(() => {
    vi.restoreAllMocks();
    delete (Element.prototype as Partial<Element>).scrollIntoView;
  });

  test("pressing the feature's row after a story was chosen fits the whole drawing again", async () => {
    fakeApi({ status: 200, body: { nodes: testTree() } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const opening = await openingView();

    // An arrow key with nothing chosen chooses the first story; keys move the view at once.
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    const story = currentView();
    expect(story.k).toBeGreaterThan(opening.k * 2);

    fireEvent.click(rowOf("feature")!);
    await waitFor(() => expect(currentView().k).toBeLessThan(story.k));
    const view = await landed();
    expect(rowOf("feature")!.getAttribute("aria-current")).toBe("true");
    expect(view.k).toBeCloseTo(opening.k, 6);
    expect(view.x).toBeCloseTo(opening.x, 6);
    expect(view.y).toBeCloseTo(opening.y, 6);
  });

  test("pressing the text of a closed story's row opens it as well as choosing it", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();

    fireEvent.click(row(aside, "Player: create an account and get set up"));

    expect(chevron(aside, "Player: create an account and get set up")!.getAttribute("aria-expanded")).toBe("true");
    expect(rowTexts(aside)).toEqual([
      "Disc golf putting trainer",
      "Player: create an account and get set up",
      "Player signs up with an email",
      "Player skips the setup",
      "Player: pick a training program",
    ]);
    expect(row(aside, "Player: create an account and get set up").getAttribute("aria-current")).toBe("true");
  });

  test("pressing a box chooses it in the folders and the details, inks the lines up to its story and leaves the graph still", async () => {
    // jsdom has no scrollIntoView; the spy is removed again after the test.
    const scrolled = vi.fn();
    Element.prototype.scrollIntoView = scrolled;
    fakeApi({ status: 200, body: { nodes: testTree() } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const opening = await openingView();

    press(boxOf(CANNOT_REMEMBER));

    expect(boxOf(CANNOT_REMEMBER).hasAttribute("data-chosen")).toBe(true);
    expect(document.querySelectorAll("main [data-chosen]")).toHaveLength(1);
    expect(rowOf(CANNOT_REMEMBER)!.getAttribute("aria-current")).toBe("true");
    expect(scrolled.mock.contexts.at(-1)).toBe(rowOf(CANNOT_REMEMBER));
    expect(scrolled.mock.calls.at(-1)).toEqual([{ block: "nearest" }]);
    expect(detailsTitle()).toBe("Player can't remember how to sign in");
    // Two lines lead up to the story: into this node and into the node above it.
    expect(document.querySelectorAll('main svg path[stroke="var(--foreground)"]')).toHaveLength(2);
    // Any eased move would have landed by now: 240ms and the 120ms fallback after it.
    await new Promise((resolve) => setTimeout(resolve, 500));
    expect(currentView()).toEqual(opening);
  });

  test("pressing the graph's background or the chosen box again keeps the choice", async () => {
    fakeApi({ status: 200, body: { nodes: testTree() } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    await openingView();
    press(boxOf(CANNOT_REMEMBER));

    press(graphArea());
    expect(boxOf(CANNOT_REMEMBER).hasAttribute("data-chosen")).toBe(true);
    expect(detailsTitle()).toBe("Player can't remember how to sign in");

    press(boxOf(CANNOT_REMEMBER));
    expect(boxOf(CANNOT_REMEMBER).hasAttribute("data-chosen")).toBe(true);
    expect(detailsTitle()).toBe("Player can't remember how to sign in");
  });

  test("double-pressing a box fits that box and everything under it in the graph area", async () => {
    fakeApi({ status: 200, body: { nodes: testTree() } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const opening = await openingView();
    const below = [...document.querySelectorAll<HTMLElement>("main [data-node]")]
      .filter((box) => box.dataset.node === SEARCHES || box.dataset.node!.startsWith(`${SEARCHES}/`))
      .map(rectOf);
    const left = Math.min(...below.map((b) => b.x));
    const top = Math.min(...below.map((b) => b.y));
    const right = Math.max(...below.map((b) => b.x + b.w));
    const bottom = Math.max(...below.map((b) => b.y + b.h));

    press(boxOf(SEARCHES));
    press(boxOf(SEARCHES));

    await waitFor(() => expect(currentView().k).toBeGreaterThan(opening.k * 2));
    const view = await landed();
    expect(view.k).toBeLessThanOrEqual(1);
    expect(view.x + left * view.k).toBeGreaterThanOrEqual(0);
    expect(view.y + top * view.k).toBeGreaterThanOrEqual(0);
    expect(view.x + right * view.k).toBeLessThanOrEqual(AREA.w);
    expect(view.y + bottom * view.k).toBeLessThanOrEqual(AREA.h);
  });

  test("the arrow keys choose the parent, the first child and the brothers and sisters, and do nothing at an edge", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const key = (name: string) => fireEvent.keyDown(document.body, { key: name });
    const story = "As a player, I want to create an account and get set up, so that my practice is saved";

    key("ArrowDown");
    expect(detailsTitle()).toBe(story);
    key("ArrowRight");
    expect(detailsTitle()).toBe("Player signs up with an email");
    key("ArrowRight");
    expect(detailsTitle()).toBe("Player confirms the email");
    key("ArrowRight");
    expect(detailsTitle()).toBe("Player confirms the email");
    key("ArrowLeft");
    expect(detailsTitle()).toBe("Player signs up with an email");
    key("ArrowDown");
    expect(detailsTitle()).toBe("Player skips the setup");
    key("ArrowDown");
    expect(detailsTitle()).toBe("Player skips the setup");
    key("ArrowUp");
    expect(detailsTitle()).toBe("Player signs up with an email");
    key("ArrowUp");
    expect(detailsTitle()).toBe("Player signs up with an email");
    key("ArrowLeft");
    expect(detailsTitle()).toBe(story);
    // Left on a story does not choose the feature.
    key("ArrowLeft");
    expect(detailsTitle()).toBe(story);
    key("ArrowDown");
    expect(detailsTitle()).toBe("As a player, I want to pick a training program, so that I know what to practise");
  });

  test("the arrow keys do nothing in a field or with ⌘, Ctrl or Alt held, and only a key that chooses stops the page scrolling", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const field = document.createElement("input");
    document.body.append(field);
    field.focus();

    expect(fireEvent.keyDown(field, { key: "ArrowDown" })).toBe(true);
    expect(fireEvent.keyDown(document.body, { key: "ArrowDown", metaKey: true })).toBe(true);
    expect(fireEvent.keyDown(document.body, { key: "ArrowDown", ctrlKey: true })).toBe(true);
    expect(fireEvent.keyDown(document.body, { key: "ArrowDown", altKey: true })).toBe(true);
    expect(detailsTitle()).toBeNull();

    // fireEvent returns false when the screen called preventDefault.
    expect(fireEvent.keyDown(document.body, { key: "ArrowDown" })).toBe(false);
    expect(detailsTitle()).toBe("As a player, I want to create an account and get set up, so that my practice is saved");
    expect(fireEvent.keyDown(document.body, { key: "ArrowLeft" })).toBe(true);
    field.remove();
  });
});
