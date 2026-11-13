// @vitest-environment jsdom
// US-221: a feature's Behaviour screen, .../features/<feature>/behaviour. It
// reads GET .../features/<feature>/behaviour and lists the tree as folders: the
// feature first, then its stories, closed, by their short titles. An empty list
// is a feature with no tree; any failed request, 404 included, is a failed load.
// The app is rendered with its own router on a memory history and the
// browser's fetch is replaced by a fake of the API.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
function node(
  id: string,
  parentId: string | null,
  title: string,
  fields: { status?: string; position?: number; default_answers?: number; needs_pass?: boolean } = {},
) {
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
    default_answers: fields.default_answers ?? 0,
    needs_pass: fields.needs_pass ?? false,
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
 * Reading a node in full answers from `reads`, keyed by node id.
 */
function fakeApi(behaviour: Behaviour, features: unknown[] | "pending" = [], questions?: unknown[], reads: Record<string, unknown> = {}) {
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
          if (path.startsWith(`${API}/features/${FEATURE_ID}/behaviour/nodes/`)) {
            const read = reads[decodeURIComponent(path.split("/").at(-1)!)];
            if (read) return json(200, read);
          }
          return json(404, { error: "not_found" });
      }
    }),
  );
}

/** A WebSocket that stays connecting until a test opens it, as the live updates do. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  sent: unknown[] = [];
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(readonly url: string) {
    super();
    FakeWebSocket.instances.push(this);
  }

  send(data: string) {
    if (this.readyState !== FakeWebSocket.OPEN) throw new Error("InvalidStateError: the socket is not open");
    this.sent.push(JSON.parse(data));
  }

  close() {
    this.readyState = FakeWebSocket.CLOSED;
  }

  /** The server accepted the connection. */
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.dispatchEvent(new Event("open"));
  }

  /** The server sent a JSON text frame. */
  receive(message: unknown) {
    this.dispatchEvent(Object.assign(new Event("message"), { data: JSON.stringify(message) }));
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
  FakeWebSocket.instances = [];
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

  // US-232: folds belong to the graph. A fold pill hides what lies under its box
  // and the pressed box stays where it is on screen; choosing a node a fold
  // hides undoes that fold. While the chosen node is hidden no box carries the
  // chosen look and no line toward it is inked.
  describe("folding the graph", () => {
    const FRIEND_STORY = "player-adds-friend";
    const NO_PLAYER = `${SEARCHES}/no-player-found`;
    const inkLines = () => document.querySelectorAll('main svg path[stroke="var(--foreground)"]');
    const boxCount = () => document.querySelectorAll("main [data-node]").length;
    const pillOf = (id: string) => document.querySelector<HTMLElement>(`main [data-node="${id}"] [data-fold]`)!;
    const foldButton = () => screen.getByRole("button", { name: /^(Fold|Unfold) all stories$/ });

    /** Presses a fold pill the way a pointer does: down, up and click. */
    function pressPill(id: string) {
      press(pillOf(id));
      fireEvent.click(pillOf(id));
    }

    test("folding the box above the chosen node takes away its chosen look and ink lines, and unfolding brings them back", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      await openingView();
      press(boxOf(NO_PLAYER));

      pressPill(SEARCHES);

      expect(pillOf(SEARCHES).textContent).toBe("+7");
      expect(document.querySelectorAll("main [data-chosen]")).toHaveLength(0);
      expect(inkLines()).toHaveLength(0);
      expect(detailsTitle()).toBe("No username matches the search");
      expect(rowOf(NO_PLAYER)!.getAttribute("aria-current")).toBe("true");

      pressPill(SEARCHES);

      expect(boxOf(NO_PLAYER).hasAttribute("data-chosen")).toBe(true);
      expect(inkLines()).toHaveLength(2);
    });

    test("pressing the row of a folded box that a fold above hides undoes only the fold above", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      const aside = await folders();
      await openingView();
      // Opening a folder changes no box.
      fireEvent.click(within(rowOf(FRIEND_STORY)!).getByRole("button"));
      expect(boxCount()).toBe(322);
      const rowsBefore = rowTexts(aside);

      pressPill(SEARCHES);
      pressPill(FRIEND_STORY);

      // Folding changes no row in the folders.
      expect(rowTexts(aside)).toEqual(rowsBefore);
      expect(pillOf(FRIEND_STORY).textContent).toBe("+32");
      expect(boxCount()).toBe(322 - 32);

      fireEvent.click(rowOf(SEARCHES)!);

      expect(pillOf(FRIEND_STORY).textContent).toBe("−");
      expect(pillOf(SEARCHES).textContent).toBe("+7");
      expect(boxCount()).toBe(322 - 7);
      expect(boxOf(SEARCHES).hasAttribute("data-chosen")).toBe(true);
    });

    test("the pressed box stays at the same place on screen in every frame of the fold", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      await openingView();
      const onScreen = () => {
        const view = currentView();
        const box = rectOf(boxOf(FRIEND_STORY));
        return { x: view.x + box.x * view.k, y: view.y + box.y * view.k };
      };
      const inDrawing = rectOf(boxOf(FRIEND_STORY));
      const before = onScreen();
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
      try {
        pressPill(FRIEND_STORY);
        const frames: { x: number; y: number }[] = [onScreen()];
        for (let t = 0; t < 400; t += 16) {
          act(() => vi.advanceTimersByTime(16));
          frames.push(onScreen());
        }

        // The box moves in the drawing, so the view has to follow it.
        expect(rectOf(boxOf(FRIEND_STORY)).y).not.toBeCloseTo(inDrawing.y, 0);
        const off = frames.map((at) => Math.max(Math.abs(at.x - before.x), Math.abs(at.y - before.y)));
        expect(Math.max(...off)).toBeLessThan(0.5);
      } finally {
        vi.useRealTimers();
      }
    });

    test("Fold all stories leaves the 13 stories and eases to fit them, and Unfold all stories brings back all 322 and the whole drawing", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      const opening = await openingView();

      fireEvent.click(screen.getByRole("button", { name: "Fold all stories" }));

      expect(boxCount()).toBe(13);
      expect(foldButton().textContent).toBe("Unfold all stories");
      // An eased move has not started yet; a jump would have moved the view already.
      expect(currentView()).toEqual(opening);
      await waitFor(() => expect(currentView().k).toBeGreaterThan(opening.k));
      const view = await landed();
      for (const box of document.querySelectorAll<HTMLElement>("main [data-node]")) {
        const rect = rectOf(box);
        expect(view.x + rect.x * view.k).toBeGreaterThanOrEqual(0);
        expect(view.y + rect.y * view.k).toBeGreaterThanOrEqual(0);
        expect(view.x + (rect.x + rect.w) * view.k).toBeLessThanOrEqual(AREA.w);
        expect(view.y + (rect.y + rect.h) * view.k).toBeLessThanOrEqual(AREA.h);
      }

      fireEvent.click(foldButton());

      expect(boxCount()).toBe(322);
      expect(foldButton().textContent).toBe("Fold all stories");
      await waitFor(() => expect(currentView().k).toBeLessThan(view.k));
      const whole = await landed();
      expect(whole.k).toBeCloseTo(opening.k, 6);
      expect(whole.x).toBeCloseTo(opening.x, 6);
      expect(whole.y).toBeCloseTo(opening.y, 6);
    });

    test("the fold button reads Unfold all stories exactly while every story is folded, and a pill press chooses nothing", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      await openingView();
      fireEvent.click(screen.getByRole("button", { name: "Fold all stories" }));

      pressPill(FRIEND_STORY);

      expect(foldButton().textContent).toBe("Fold all stories");
      expect(boxCount()).toBe(13 + 32);
      expect(document.querySelectorAll("main [data-chosen]")).toHaveLength(0);
      expect(detailsTitle()).toBeNull();

      pressPill(FRIEND_STORY);

      expect(foldButton().textContent).toBe("Unfold all stories");

      // P40: an inner fold stays when the fold above it is opened again.
      fireEvent.click(foldButton());
      pressPill(SEARCHES);
      pressPill(FRIEND_STORY);
      pressPill(FRIEND_STORY);

      expect(pillOf(SEARCHES).textContent).toBe("+7");
      expect(boxCount()).toBe(322 - 7);
    });
  });

  // US-233: a field above the folders searches the tree. While its query holds
  // a word the folders give way to the nodes found, in tree order, and the
  // graph dims the others without moving.
  describe("searching the tree", () => {
    const PUTTS = "0192d7a0-8888-7000-8000-000000000007";
    const STORY = "Player: create an account and get set up";
    const SET_UP_STORY = "player-sets-up-account";
    // "invite link" in the test tree's titles: the first, second and last of the 11 found.
    const FIRST = "player-sets-up-account/player-opens-friend-invite-link";
    const SECOND = `${FIRST}/invite-not-recognized`;
    const LAST = "player-adds-friend/player-opens-invite-link/player-is-offline";

    const field = () => screen.getByRole("textbox", { name: "Search the tree" }) as HTMLInputElement;
    const type = (text: string) => fireEvent.change(field(), { target: { value: text } });
    const fieldKey = (key: string) => fireEvent.keyDown(field(), { key });
    const marks = (result: HTMLElement) => [...result.querySelectorAll("mark")].map((mark) => mark.textContent);
    const muted = (element: HTMLElement) => element.className.split(/\s+/).includes("opacity-50");

    test("a query lists the nodes found in tree order by their short titles, each typed word marked and the path above beneath", async () => {
      fakeApi({ status: 200, body: { nodes: [...TREE, node(PUTTS, PROGRAM_STORY, "Player putts again (once more)")] } });
      renderAt(BEHAVIOUR_PATH);
      const aside = await folders();

      type("EMAIL player");

      expect(screen.getByText("2 found")).toBeTruthy();
      const [signsUp, confirms] = rows(aside);
      expect(rows(aside)).toHaveLength(2);
      expect(signsUp.firstElementChild!.textContent).toBe("Player signs up with an email");
      expect(marks(signsUp)).toEqual(["Player", "email"]);
      expect(signsUp.lastElementChild!.textContent).toBe(STORY);
      expect(confirms.firstElementChild!.textContent).toBe("Player confirms the email");
      expect(confirms.lastElementChild!.textContent).toBe(`${STORY} › Player signs up with an email`);

      type("program");
      expect(rows(aside)).toHaveLength(1);
      expect(rows(aside)[0].firstElementChild!.textContent).toBe("Player: pick a training program");
      expect(marks(rows(aside)[0])).toEqual(["program"]);
      expect(rows(aside)[0].lastElementChild!.textContent).toBe("Feature");

      // The typed words are marked as written, not read as a pattern.
      type("(once");
      expect(marks(rows(aside)[0])).toEqual(["(once"]);

      fireEvent.click(rows(aside)[0]);
      expect(rows(aside)[0].getAttribute("aria-current")).toBe("true");
      expect(rows(aside)).toHaveLength(1);
      expect(detailsTitle()).toBe("Player putts again (once more)");
    });

    test("a query of only spaces keeps the folders, and words that find nothing say so", async () => {
      fakeApi({ status: 200, body: { nodes: TREE } });
      renderAt(BEHAVIOUR_PATH);
      const aside = await folders();
      const before = rowTexts(aside);

      type("   ");
      expect(rowTexts(aside)).toEqual(before);

      type("zebra");
      expect(screen.getByText("0 found")).toBeTruthy();
      expect(rowTexts(aside)).toEqual(['Nothing in the tree matches "zebra".']);
    });

    test("arrow down, arrow up and Enter in the field step through the results, wrapping at both ends, and jump the view", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      await openingView();
      press(boxOf(SET_UP_STORY));
      type("invite link");
      expect(screen.getByText("11 found")).toBeTruthy();

      /** Presses `key` in the field and returns what it chose, checking the view moved at once. */
      const step = (key: string) => {
        const before = currentView();
        fieldKey(key);
        expect(currentView()).not.toEqual(before);
        return document.querySelector<HTMLElement>("aside [aria-current]")?.dataset.row;
      };

      // From a chosen node outside the results, up goes to the last and down to the first.
      expect(step("ArrowUp")).toBe(LAST);
      expect(step("ArrowDown")).toBe(FIRST);
      expect(step("Enter")).toBe(SECOND);
      expect(step("ArrowUp")).toBe(FIRST);
      expect(step("ArrowUp")).toBe(LAST);
      press(boxOf(SET_UP_STORY));
      expect(step("ArrowDown")).toBe(FIRST);
      expect(document.querySelectorAll("aside [data-result]")).toHaveLength(11);

      // With nothing found the keys choose nothing.
      type("zebra");
      const view = currentView();
      fieldKey("ArrowDown");
      fieldKey("Enter");
      expect(currentView()).toEqual(view);
      expect(detailsTitle()).toBe("Player opens a friend's invite link without having an account");
    });

    test("while a query is typed the graph stays still and dims the boxes not found", async () => {
      fakeApi({ status: 200, body: { nodes: testTree() } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      const opening = await openingView();

      type("invite link");

      expect(muted(boxOf(FIRST))).toBe(false);
      expect(muted(boxOf(SET_UP_STORY))).toBe(true);
      // Any eased move would have landed by now.
      await new Promise((resolve) => setTimeout(resolve, 500));
      expect(currentView()).toEqual(opening);
    });

    test("the key / puts the cursor in the field with its text selected, unless a field has focus or a modifier is held", async () => {
      fakeApi({ status: 200, body: { nodes: TREE } });
      renderAt(BEHAVIOUR_PATH);
      await folders();
      type("email");
      field().blur();
      const other = document.body.appendChild(document.createElement("input"));

      for (const modifier of ["metaKey", "ctrlKey", "altKey"]) fireEvent.keyDown(document.body, { key: "/", [modifier]: true });
      fireEvent.keyDown(other, { key: "/" });
      expect(document.activeElement).not.toBe(field());

      // fireEvent returns false when the key's default, typing "/", was stopped.
      expect(fireEvent.keyDown(document.body, { key: "/" })).toBe(false);
      expect(document.activeElement).toBe(field());
      expect([field().selectionStart, field().selectionEnd]).toEqual([0, 5]);
      other.remove();
    });

    test("Esc in the field and the clear button bring the folders back with the chosen row in view, and Esc elsewhere clears the query", async () => {
      const scrolled = vi.fn();
      Element.prototype.scrollIntoView = scrolled;
      fakeApi({ status: 200, body: { nodes: TREE } });
      renderAt(BEHAVIOUR_PATH);
      const aside = await folders();
      field().focus();
      type("email");
      fieldKey("ArrowDown");
      expect(rowOf(SIGNS_UP)!.getAttribute("aria-current")).toBe("true");

      fieldKey("Escape");
      expect(field().value).toBe("");
      expect(document.activeElement).not.toBe(field());
      expect(rowTexts(aside)).toContain("Player signs up with an email");
      expect(scrolled.mock.contexts.at(-1)).toBe(rowOf(SIGNS_UP));

      field().focus();
      type("email");
      // A press moves the focus to the button, as a pointer would.
      const clear = screen.getByRole("button", { name: "Clear search" });
      clear.focus();
      fireEvent.click(clear);
      expect(field().value).toBe("");
      expect(document.activeElement).toBe(field());
      expect(rowTexts(aside)).toContain("Player signs up with an email");
      expect(scrolled.mock.contexts.at(-1)).toBe(rowOf(SIGNS_UP));

      type("email");
      field().blur();
      fireEvent.keyDown(document.body, { key: "Escape" });
      expect(field().value).toBe("");
    });
  });

  // US-235: a behaviour event of this feature on the project's live socket reads
  // the tree and the open questions again, at most once per 300 ms: on the first
  // event, and once more at the end of the window when more came in it. The
  // chosen node is read again when the event names it, or is behaviour.imported
  // or behaviour.marked.
  describe("following a running grill", () => {
    const OTHER_FEATURE = "0192d7a0-4444-7000-8000-000000000002";
    const pillOf = (id: string) => document.querySelector<HTMLElement>(`main [data-node="${id}"] [data-fold]`)!;
    const still = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

    /** How many times `path` was requested. */
    function requests(path: string) {
      return vi.mocked(globalThis.fetch).mock.calls.filter(([input]) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        return new URL(url, "http://localhost:3000").pathname === path;
      }).length;
    }
    const treeReads = () => requests(`${API}/features/${FEATURE_ID}/behaviour`);
    const questionReads = () => requests(`${API}/features/${FEATURE_ID}/behaviour/questions`);
    const nodeReads = (id: string) => requests(`${API}/features/${FEATURE_ID}/behaviour/nodes/${id}`);

    /** Opens the live socket and returns a way to send a project event on it. */
    async function liveSocket() {
      await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
      const socket = FakeWebSocket.instances.at(-1)!;
      act(() => socket.open());
      await waitFor(() => expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })));
      act(() => socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 0 }));
      let seq = 0;
      return (event: string, data: Record<string, unknown>) =>
        act(() => {
          seq += 1;
          socket.receive({ type: "event", seq, event, project_id: PROJECT_ID, at: AT, data });
        });
    }

    function readOf(id: string, parentId: string, title: string) {
      return {
        node: { ...node(id, parentId, title), level: 2, situation: "", behaviour: "", created_at: AT, created_by: BY_AGENT, updated_at: AT },
        chain: [],
        children: [],
        siblings: [],
        questions: [],
        rules: [],
      };
    }

    test("a burst of this feature's behaviour events reads the tree and the open questions at once and once more at its end, and a lone event once", async () => {
      const list = { status: 200, body: { nodes: TREE } };
      fakeApi(list, [], []);
      renderAt(BEHAVIOUR_PATH);
      const aside = await folders();
      const send = await liveSocket();
      await waitFor(() => expect(questionReads()).toBe(1));
      expect(treeReads()).toBe(1);

      send("behaviour.node_changed", { feature_id: FEATURE_ID, node_id: PROGRAM_STORY });
      await waitFor(() => expect(treeReads()).toBe(2));
      expect(questionReads()).toBe(2);
      await still(450);
      expect(treeReads()).toBe(2);
      expect(questionReads()).toBe(2);

      send("behaviour.node_changed", { feature_id: FEATURE_ID, node_id: PROGRAM_STORY });
      await waitFor(() => expect(treeReads()).toBe(3));
      const STATS_STORY = "0192d7a0-8888-7000-8000-000000000007";
      list.body = { nodes: [...TREE, node(STATS_STORY, ROOT, "As a player, I want to see my putting stats, so that I know where I improve", { position: 2 })] };
      send("behaviour.node_created", { feature_id: FEATURE_ID, node_id: STATS_STORY });
      send("behaviour.question_changed", { feature_id: FEATURE_ID, node_id: STATS_STORY, question_id: "q9", state: "open" });

      await waitFor(() => expect(within(aside).queryByText("Player: see my putting stats")).not.toBeNull());
      expect(treeReads()).toBe(4);
      expect(questionReads()).toBe(4);
      await still(450);
      expect(treeReads()).toBe(4);
      expect(questionReads()).toBe(4);
    }, 10_000);

    test("an event of another feature of the project, or one that is not about behaviour, reads nothing of this feature", async () => {
      fakeApi({ status: 200, body: { nodes: TREE } }, [], []);
      renderAt(BEHAVIOUR_PATH);
      await folders();
      const send = await liveSocket();
      await waitFor(() => expect(questionReads()).toBe(1));

      send("behaviour.node_created", { feature_id: OTHER_FEATURE, node_id: "0192d7a0-8888-7000-8000-000000000099" });
      send("feature.renamed", { feature_id: FEATURE_ID, name: "Putting" });
      await still(450);
      expect(treeReads()).toBe(1);
      expect(questionReads()).toBe(1);

      send("behaviour.node_created", { feature_id: FEATURE_ID, node_id: SIGNS_UP });
      await waitFor(() => expect(treeReads()).toBe(2));
    }, 10_000);

    test("the chosen node is read again for an event that names it, for behaviour.marked and behaviour.imported, and at the end of a burst one of them came in, keeping the typed remark", async () => {
      fakeApi({ status: 200, body: { nodes: TREE } }, [], [], { [SIGNS_UP]: readOf(SIGNS_UP, SIGN_UP_STORY, TREE[2].title) });
      renderAt(BEHAVIOUR_PATH);
      const aside = await folders();
      const send = await liveSocket();
      fireEvent.click(chevron(aside, "Player: create an account and get set up") as HTMLElement);
      fireEvent.click(within(aside).getByText("Player signs up with an email"));
      await waitFor(() => expect(nodeReads(SIGNS_UP)).toBe(1));
      const remark = (await screen.findByRole("textbox", { name: "Add a question or remark" })) as HTMLTextAreaElement;
      fireEvent.change(remark, { target: { value: "The confirmation is easy to miss." } });

      send("behaviour.node_changed", { feature_id: FEATURE_ID, node_id: CONFIRMS });
      await waitFor(() => expect(treeReads()).toBe(2));
      await still(450);
      expect(nodeReads(SIGNS_UP)).toBe(1);

      send("behaviour.question_changed", { feature_id: FEATURE_ID, node_id: SIGNS_UP, question_id: "q1", state: "answered" });
      await waitFor(() => expect(nodeReads(SIGNS_UP)).toBe(2));
      await still(450);

      send("behaviour.marked", { feature_id: FEATURE_ID, node_id: CONFIRMS, below: false });
      await waitFor(() => expect(nodeReads(SIGNS_UP)).toBe(3));
      await still(450);

      send("behaviour.imported", { feature_id: FEATURE_ID });
      await waitFor(() => expect(nodeReads(SIGNS_UP)).toBe(4));
      await still(450);

      send("behaviour.node_changed", { feature_id: FEATURE_ID, node_id: CONFIRMS });
      send("behaviour.question_changed", { feature_id: FEATURE_ID, node_id: SIGNS_UP, question_id: "q1", state: "answered" });
      await waitFor(() => expect(nodeReads(SIGNS_UP)).toBe(5));
      await still(450);
      expect(nodeReads(SIGNS_UP)).toBe(5);
      expect(remark.value).toBe("The confirmation is easy to miss.");
      expect(screen.getByRole("textbox", { name: "Add a question or remark" })).toBe(remark);
    }, 10_000);

    test("a node added live shows as a box and under its open row while the choice, the view and the folds stay, and a row given its first child starts closed", async () => {
      const STORY = "player-sets-up-account";
      const FIRST_TIME = `${STORY}/player-opens-app-first-time`;
      const NO_CONNECTION = `${FIRST_TIME}/no-connection`;
      const PROGRAM = "player-picks-program";
      const OFFLINE = `${STORY}/player-is-offline`;
      const NO_SIGNAL = `${NO_CONNECTION}/player-has-no-signal`;
      const PROGRAM_OFFLINE = `${PROGRAM}/player-opens-program-offline`;
      const tree = testTree();
      const list = { status: 200, body: { nodes: tree } };
      fakeApi(list);
      renderAt(BEHAVIOUR_PATH);
      await folders();
      await openingView();
      const send = await liveSocket();
      press(boxOf(STORY));
      fireEvent.click(within(rowOf(STORY)!).getByRole("button"));
      fireEvent.click(within(rowOf(FIRST_TIME)!).getByRole("button"));
      expect(within(rowOf(NO_CONNECTION)!).queryByRole("button")).toBeNull();
      const unfolded = currentView();
      press(pillOf(PROGRAM));
      fireEvent.click(pillOf(PROGRAM));
      expect(pillOf(PROGRAM).textContent).toBe("+27");
      // The fold moves the view to keep the pressed box still; wait for that move to land.
      await waitFor(() => expect(currentView()).not.toEqual(unfolded));
      const before = await landed();

      const added = (id: string, parentId: string, title: string, position: number) => ({ ...tree[0], id, parent_id: parentId, path: id, position, level: id.split("/").length, title });
      const at = (id: string) => tree.findIndex((one) => one.id === id);
      list.body = {
        nodes: [
          ...tree.slice(0, at(NO_CONNECTION) + 1),
          added(NO_SIGNAL, NO_CONNECTION, "Player has no signal", 0),
          ...tree.slice(at(NO_CONNECTION) + 1, at(PROGRAM)),
          added(OFFLINE, STORY, "Player is offline", 3),
          ...tree.slice(at(PROGRAM)),
          added(PROGRAM_OFFLINE, PROGRAM, "Player opens a program offline", 9),
        ],
      };
      send("behaviour.node_created", { feature_id: FEATURE_ID, node_id: OFFLINE });

      await waitFor(() => expect(boxOf(OFFLINE)).not.toBeNull(), { timeout: 2000 });
      expect(rowOf(OFFLINE)!.textContent).toBe("Player is offline");
      expect(within(rowOf(NO_CONNECTION)!).getByRole("button").getAttribute("aria-expanded")).toBe("false");
      expect(rowOf(NO_SIGNAL)).toBeNull();
      expect(boxOf(STORY).hasAttribute("data-chosen")).toBe(true);
      expect(rowOf(STORY)!.getAttribute("aria-current")).toBe("true");
      expect(pillOf(PROGRAM).textContent).toBe("+28");
      expect(boxOf(PROGRAM_OFFLINE)).toBeNull();
      await still(500);
      expect(currentView()).toEqual(before);
    }, 10_000);
  });
});

describe("the Behaviour screen's details", () => {
  test("name the signed-in member's own answer You and another member's A member", async () => {
    const answer = (text: string, userId: string) => ({ answer: text, at: AT, author: { kind: "human", user_id: userId }, given_by: "human" });
    const answered = (id: string, text: string, given: ReturnType<typeof answer>) => ({
      id,
      node_id: SIGN_UP_STORY,
      node_path: "",
      question: text,
      state: "answered",
      asked_by: BY_AGENT,
      answer: given,
      answers: [given],
      created_at: AT,
      version: 1,
    });
    const read = {
      node: { ...node(SIGN_UP_STORY, ROOT, TREE[1].title), level: 1, situation: "", behaviour: "", created_at: AT, created_by: BY_AGENT, updated_at: AT },
      chain: [],
      children: [],
      siblings: [],
      questions: [
        answered("q-mine", "Which putt counts?", answer("Inside the circle", USER.id)),
        answered("q-theirs", "Who keeps score?", answer("The player", "0192d7a0-1111-7000-8000-000000000002")),
      ],
      rules: [],
    };
    fakeApi({ status: 200, body: { nodes: TREE } }, [], [], { [SIGN_UP_STORY]: read });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();

    fireEvent.click(row(aside, "Player: create an account and get set up"));

    const details = screen.getByRole("region", { name: "Details" });
    await waitFor(() => expect(details.querySelector('[data-question="q-mine"]')).not.toBeNull());
    const mine = details.querySelector<HTMLElement>('[data-question="q-mine"]')!;
    expect(mine.textContent).toContain("You");
    expect(mine.textContent).not.toContain("A member");
    expect(details.querySelector('[data-question="q-theirs"]')!.textContent).toContain("A member");
  });
});

// US-228: after a person's answer, accepted or turned away with a 409, the
// screen reads the tree, the open questions and the chosen node again.
describe("answering a waiting question in the Behaviour screen", () => {
  const ANSWERS_PATH = `${API}/features/${FEATURE_ID}/behaviour/questions/q1/answers`;
  const STORY = "Player: create an account and get set up";
  const given = (text: string, userId: string) => ({ answer: text, at: AT, author: { kind: "human", user_id: userId }, given_by: "human" });
  const answered = (text: string, userId: string) => ({ ...openQuestion("q1", SIGNS_UP, BY_AGENT), state: "answered", answer: given(text, userId), answers: [given(text, userId)], version: 2 });

  function readOf(questions: unknown[]) {
    return {
      node: { ...node(SIGNS_UP, SIGN_UP_STORY, TREE[2].title), level: 2, situation: "", behaviour: "", created_at: AT, created_by: BY_AGENT, updated_at: AT },
      chain: [],
      children: [],
      siblings: [],
      questions,
      rules: [],
    };
  }

  /**
   * The tree with one question an agent asked on "Player signs up with an
   * email" and one on the node below it. When an answer is posted, the server
   * has moved on (the node needs another pass, its question is answered with
   * `answer`) and the POST answers with `status`.
   */
  function fakeAnswering(status: number, answer: { text: string; by: string }) {
    const list = { status: 200, body: { nodes: TREE } };
    const questions = [openQuestion("q1", SIGNS_UP, BY_AGENT), openQuestion("q2", CONFIRMS, BY_AGENT)];
    const reads: Record<string, unknown> = { [SIGNS_UP]: readOf([questions[0]]) };
    fakeApi(list, [], questions, reads);
    const api = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (new URL(url, "http://localhost:3000").pathname !== ANSWERS_PATH || init?.method !== "POST") return api(input, init);
        const now = answered(answer.text, answer.by);
        list.body = { nodes: TREE.map((one) => (one.id === SIGNS_UP ? { ...one, needs_pass: true, needs_pass_since: AT, status: "open" } : one)) };
        questions.splice(0, 1);
        reads[SIGNS_UP] = readOf([now]);
        const body = status === 201 ? { question: now } : { error: "already_answered" };
        return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
      }),
    );
  }

  const badge = (aside: HTMLElement) => rows(aside).find((one) => within(one).queryByText(STORY))!.querySelector('[data-mark="waiting"]');

  /** Chooses "Player signs up with an email", closes its story, types an answer and presses Answer. */
  async function answerSignsUp() {
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    const storyRow = () => rows(aside).find((one) => within(one).queryByText(STORY))!;
    fireEvent.click(within(storyRow()).getByRole("button"));
    fireEvent.click(within(aside).getByText("Player signs up with an email"));
    fireEvent.click(within(storyRow()).getByRole("button"));
    await waitFor(() => expect(badge(aside)?.textContent).toBe("2"));
    const details = screen.getByRole("region", { name: "Details" });
    const field = await within(details).findByRole("textbox", { name: "Your answer" });
    fireEvent.change(field, { target: { value: "Name and handicap" } });
    fireEvent.click(within(details).getByRole("button", { name: "Answer" }));
    return { aside, details };
  }

  test("an accepted answer shows the node needing another grill pass and drops the closed story's count by one", async () => {
    fakeAnswering(201, { text: "Name and handicap", by: USER.id });
    const { aside, details } = await answerSignsUp();

    await waitFor(() => expect(within(details).queryByText("↻ Needs another grill pass")).not.toBeNull());
    expect(details.textContent).toContain("Nothing is rewritten until the grill is started again on the feature.");
    await waitFor(() => expect(badge(aside)?.textContent).toBe("1"));
    expect(details.querySelector('[data-question="q1"]')!.textContent).toContain("You");
  });

  test("a 409 reads the node again and shows the other member's answer, and the tree and the counts follow it", async () => {
    fakeAnswering(409, { text: "Name and club", by: "0192d7a0-1111-7000-8000-000000000002" });
    const { aside, details } = await answerSignsUp();

    await waitFor(() => expect(details.querySelector('[data-question="q1"]')!.textContent).toContain("Someone answered this first; it has been reloaded."));
    const q1 = details.querySelector<HTMLElement>('[data-question="q1"]')!;
    expect(q1.textContent).toContain("Name and club");
    expect(q1.textContent).toContain("A member");
    expect(within(q1).queryByRole("textbox", { name: "Your answer" })).toBeNull();
    await waitFor(() => expect(within(details).queryByText("↻ Needs another grill pass")).not.toBeNull());
    await waitFor(() => expect(badge(aside)?.textContent).toBe("1"));
  });
});

// US-230: after a person's remark the screen reads the tree, the open questions
// and the chosen node again, so the node shows the remark and needs another pass.
describe("adding a remark in the Behaviour screen", () => {
  const ASK_PATH = `${API}/features/${FEATURE_ID}/behaviour/nodes/${SIGNS_UP}/questions`;
  const REMARK = { ...openQuestion("q-remark", SIGNS_UP, BY_PERSON), question: "The confirmation is easy to miss." };

  function readOf(questions: unknown[]) {
    return {
      node: { ...node(SIGNS_UP, SIGN_UP_STORY, TREE[2].title), level: 2, situation: "", behaviour: "", created_at: AT, created_by: BY_AGENT, updated_at: AT },
      chain: [],
      children: [],
      siblings: [],
      questions,
      rules: [],
    };
  }

  /** The tree with nothing open; once a question is posted on "Player signs up with an email" it is open again and needs another pass. */
  function fakeRemarking() {
    const list = { status: 200, body: { nodes: TREE } };
    const questions: unknown[] = [];
    const reads: Record<string, unknown> = { [SIGNS_UP]: readOf([]) };
    fakeApi(list, [], questions, reads);
    const api = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (new URL(url, "http://localhost:3000").pathname !== ASK_PATH || init?.method !== "POST") return api(input, init);
        list.body = { nodes: TREE.map((one) => (one.id === SIGNS_UP ? { ...one, needs_pass: true, needs_pass_since: AT, status: "open" } : one)) };
        questions.push(REMARK);
        reads[SIGNS_UP] = readOf([REMARK]);
        return new Response(JSON.stringify({ question: REMARK }), { status: 201, headers: { "content-type": "application/json" } });
      }),
    );
  }

  afterEach(() => {
    vi.useRealTimers();
  });

  test("an added remark empties the field, shows as a Remark by You today that waits for the next grill pass, and the node needs another grill pass", async () => {
    vi.useFakeTimers({ toFake: ["Date"], now: new Date("2026-11-12T15:00:00Z") });
    fakeRemarking();
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    fireEvent.click(within(rows(aside).find((one) => within(one).queryByText("Player: create an account and get set up"))!).getByRole("button"));
    fireEvent.click(within(aside).getByText("Player signs up with an email"));
    const details = screen.getByRole("region", { name: "Details" });
    const field = (await within(details).findByRole("textbox", { name: "Add a question or remark" })) as HTMLTextAreaElement;
    expect(within(details).queryByText("↻ Needs another grill pass")).toBeNull();

    fireEvent.change(field, { target: { value: "The confirmation is easy to miss." } });
    fireEvent.click(within(details).getByRole("button", { name: "Add to this node" }));

    await waitFor(() => expect(details.querySelector('[data-question="q-remark"]')).not.toBeNull());
    const remark = details.querySelector<HTMLElement>('[data-question="q-remark"]')!;
    expect(within(remark).getByText("Remark")).toBeTruthy();
    expect(remark.textContent).toContain("You · today · waits for the next grill pass");
    expect(within(remark).queryByRole("textbox", { name: "Your answer" })).toBeNull();
    expect(field.value).toBe("");
    await waitFor(() => expect(within(details).queryByText("↻ Needs another grill pass")).not.toBeNull());
  });
});

// US-231: one filter at a time cuts the tree down to the nodes it picks out
// and every node above them, the others muted; the chosen node always stays.
// A filter change fits the graph again at once. The top bar's sentence counts
// the questions agents asked (never remarks), the answers given by default and
// the nodes waiting for the next grill pass, leaving out a part at 0.
describe("filtering the Behaviour screen", () => {
  const AREA = { w: 748, h: 844 };
  type View = { x: number; y: number; k: number };

  /** A situation that is open, with one below it holding 3 answers given by default. */
  const FILTERED = [
    node(ROOT, null, "Disc golf putting trainer"),
    node(SIGN_UP_STORY, ROOT, "As a player, I want to create an account and get set up, so that my practice is saved"),
    node(SIGNS_UP, SIGN_UP_STORY, "Player signs up with an email", { status: "open" }),
    node(CONFIRMS, SIGNS_UP, "Player confirms the email", { default_answers: 3 }),
    node(SKIPS, SIGN_UP_STORY, "Player skips the setup", { status: "out_of_scope", position: 1 }),
    node(PROGRAM_STORY, ROOT, "As a player, I want to pick a training program, so that I know what to practise", { position: 1 }),
  ];
  const STORY = "Player: create an account and get set up";
  const PROGRAM = "Player: pick a training program";

  const choice = (name: RegExp) => within(screen.getByRole("group", { name: "Show" })).getByRole("button", { name });
  const pressed = (name: RegExp) => choice(name).getAttribute("aria-pressed");
  const facts = () => document.querySelector("[data-facts]")?.textContent;
  const boxOf = (id: string) => document.querySelector<HTMLElement>(`main [data-node="${id}"]`);
  const boxIds = () => new Set([...document.querySelectorAll<HTMLElement>("main [data-node]")].map((box) => box.dataset.node));
  const muted = (element: HTMLElement) => element.className.split(/\s+/).includes("opacity-50");

  function currentView(): View {
    const drawing = document.querySelector<HTMLElement>('main [style*="scale("]')!;
    const [, x, y, k] = /translate\((-?[\d.e-]+)px, (-?[\d.e-]+)px\) scale\(([\d.e-]+)\)/.exec(drawing.style.transform)!;
    return { x: Number(x), y: Number(y), k: Number(k) };
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
  });

  test("Only by default opens the folders fully on the matching node and those above it, muted, and Show all brings the folders back as they were", async () => {
    fakeApi({ status: 200, body: { nodes: FILTERED } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    fireEvent.click(chevron(aside, STORY) as HTMLElement);
    const before = rowTexts(aside);
    expect(choice(/^All\s*6$/)).toBeTruthy();
    expect(choice(/^Only open\s*1$/)).toBeTruthy();

    fireEvent.click(choice(/^Only by default\s*1$/));

    expect(pressed(/^Only by default/)).toBe("true");
    expect(pressed(/^All/)).toBe("false");
    expect(rowTexts(aside)).toEqual(["Disc golf putting trainer", STORY, "Player signs up with an email", "Player confirms the email"]);
    expect([ROOT, SIGN_UP_STORY, SIGNS_UP, CONFIRMS].map((id) => muted(aside.querySelector<HTMLElement>(`[data-row="${id}"]`)!))).toEqual([
      true,
      true,
      true,
      false,
    ]);
    expect(boxIds()).toEqual(new Set([SIGN_UP_STORY, SIGNS_UP, CONFIRMS]));
    expect([SIGN_UP_STORY, SIGNS_UP, CONFIRMS].map((id) => muted(boxOf(id)!))).toEqual([true, true, false]);
    expect(screen.getByText("1 of 6 nodes, with what lies above them")).toBeTruthy();
    expect(screen.queryByLabelText("Legend")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Show all" }));

    expect(pressed(/^All/)).toBe("true");
    expect(rowTexts(aside)).toEqual(before);
    expect(screen.queryByText("1 of 6 nodes, with what lies above them")).toBeNull();
    expect(screen.getByLabelText("Legend")).toBeTruthy();
  });

  // US-233: under a filter only the nodes it picks out are found, and the count says which.
  test("with a filter on only the nodes it picks out are found, and the count names the filter", async () => {
    fakeApi({
      status: 200,
      body: {
        nodes: [
          ...FILTERED.filter((one) => one.id !== SKIPS),
          node(SKIPS, SIGN_UP_STORY, "Player skips the setup", { status: "out_of_scope", position: 1, needs_pass: true }),
        ],
      },
    });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    const found = () => rows(aside).map((one) => one.dataset.row);
    fireEvent.change(screen.getByRole("textbox", { name: "Search the tree" }), { target: { value: "player" } });
    expect(screen.getByText("5 found")).toBeTruthy();

    fireEvent.click(choice(/^Only open/));
    expect(screen.getByText("1 found in open nodes")).toBeTruthy();
    expect(found()).toEqual([SIGNS_UP]);

    fireEvent.click(choice(/^Only by default/));
    expect(screen.getByText("1 found in nodes answered by default")).toBeTruthy();
    expect(found()).toEqual([CONFIRMS]);

    fireEvent.click(choice(/^Only needing a grill pass/));
    expect(screen.getByText("1 found in nodes needing a grill pass")).toBeTruthy();
    expect(found()).toEqual([SKIPS]);
  });

  test("turning a filter on fits the drawing it shows at once, and pressing the choice that is on goes back to All and fits the whole drawing at once", async () => {
    const SEARCHES = "player-adds-friend/player-searches-for-player";
    fakeApi({ status: 200, body: { nodes: testTree().map((one) => (one.id === SEARCHES ? { ...one, default_answers: 2 } : one)) } });
    renderAt(BEHAVIOUR_PATH);
    await folders();
    await waitFor(() => expect(document.querySelector('main [style*="scale("]')).not.toBeNull());
    const opening = currentView();
    // An arrow key chooses the first story and moves the view to it at once.
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    const story = currentView();

    fireEvent.click(choice(/^Only by default/));
    const filtered = currentView();
    expect(filtered).not.toEqual(story);
    fireEvent.keyDown(document.body, { key: "f" });
    expect(currentView()).toEqual(filtered);

    fireEvent.click(choice(/^Only by default/));
    expect(pressed(/^All/)).toBe("true");
    const all = currentView();
    expect(all.k).toBeCloseTo(opening.k, 6);
    expect(all.x).toBeCloseTo(opening.x, 6);
    expect(all.y).toBeCloseTo(opening.y, 6);
  });

  test("a chosen node that does not match stays in the folders and the graph, muted under its chosen look", async () => {
    fakeApi({ status: 200, body: { nodes: FILTERED } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    fireEvent.click(row(aside, PROGRAM));

    fireEvent.click(choice(/^Only by default/));

    const programRow = aside.querySelector<HTMLElement>(`[data-row="${PROGRAM_STORY}"]`)!;
    expect(rowTexts(aside)).toEqual(["Disc golf putting trainer", STORY, "Player signs up with an email", "Player confirms the email", PROGRAM]);
    expect(programRow.getAttribute("aria-current")).toBe("true");
    expect(muted(programRow)).toBe(true);
    expect(boxOf(PROGRAM_STORY)!.hasAttribute("data-chosen")).toBe(true);
    expect(muted(boxOf(PROGRAM_STORY)!)).toBe(true);
  });

  test("a choice that picks out no node shows only the chosen node and those above it, and the line drops what lies above", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } });
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    fireEvent.click(row(aside, STORY));
    fireEvent.click(row(aside, "Player signs up with an email"));

    fireEvent.click(choice(/^Only open\s*0$/));

    expect(pressed(/^Only open/)).toBe("true");
    expect(rowTexts(aside)).toEqual(["Disc golf putting trainer", STORY, "Player signs up with an email"]);
    expect(screen.getByText("0 of 6 nodes")).toBeTruthy();
  });

  test("the top bar counts the questions agents asked, the answers given by default and the nodes waiting for a pass, and offers the grill-pass choice", async () => {
    const marked = FILTERED.map((one) => (one.id === PROGRAM_STORY ? { ...one, needs_pass: true } : one));
    fakeApi({ status: 200, body: { nodes: marked } }, [], [
      openQuestion("q1", SIGNS_UP, BY_AGENT),
      openQuestion("q2", CONFIRMS, BY_AGENT),
      openQuestion("q3", SIGNS_UP, BY_PERSON),
    ]);
    renderAt(BEHAVIOUR_PATH);
    await folders();

    await waitFor(() =>
      expect(facts()).toBe("2 questions wait for an answer · 3 answers were given by default · 1 node waits for the next grill pass"),
    );
    expect(choice(/^Only needing a grill pass\s*1$/)).toBeTruthy();
  });

  test("with nothing waiting, nothing by default and nothing marked the top bar says only that no question is open, and offers no grill-pass choice", async () => {
    fakeApi({ status: 200, body: { nodes: TREE } }, [], []);
    renderAt(BEHAVIOUR_PATH);
    await folders();

    expect(facts()).toBe("No question is open");
    expect(within(screen.getByRole("group", { name: "Show" })).queryByRole("button", { name: /^Only needing a grill pass/ })).toBeNull();
  });

  test("pressing 1 question waits for an answer turns Only open on, and pressing it again keeps it on", async () => {
    fakeApi({ status: 200, body: { nodes: FILTERED } }, [], [openQuestion("q1", SIGNS_UP, BY_AGENT)]);
    renderAt(BEHAVIOUR_PATH);
    await folders();
    const waits = await screen.findByRole("button", { name: "1 question waits for an answer" });
    expect(facts()).toBe("1 question waits for an answer · 3 answers were given by default");

    fireEvent.click(waits);
    expect(pressed(/^Only open/)).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "1 question waits for an answer" }));
    expect(pressed(/^Only open/)).toBe("true");
    expect(screen.getByText("1 of 6 nodes, with what lies above them")).toBeTruthy();

    // The details' link turns Only open on as well.
    fireEvent.click(choice(/^Only open/));
    expect(pressed(/^All/)).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Show only the open nodes" }));
    expect(pressed(/^Only open/)).toBe("true");
  });

  test("the grill-pass choice stays while it is on after the tree is read again with no node marked", async () => {
    const ASK_PATH = `${API}/features/${FEATURE_ID}/behaviour/nodes/${SIGNS_UP}/questions`;
    const list = { status: 200, body: { nodes: FILTERED.map((one) => (one.id === SIGNS_UP ? { ...one, needs_pass: true, needs_pass_since: AT } : one)) } };
    const read = {
      node: { ...FILTERED[2], level: 2, situation: "", behaviour: "", created_at: AT, created_by: BY_AGENT, updated_at: AT },
      chain: [],
      children: [],
      siblings: [],
      questions: [],
      rules: [],
    };
    fakeApi(list, [], [], { [SIGNS_UP]: read });
    const api = globalThis.fetch;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
        if (new URL(url, "http://localhost:3000").pathname !== ASK_PATH || init?.method !== "POST") return api(input, init);
        // The server has moved on: the grill ran and no node is marked any more.
        list.body = { nodes: FILTERED };
        const asked = { ...openQuestion("q-remark", SIGNS_UP, BY_PERSON), question: "Easy to miss." };
        return new Response(JSON.stringify({ question: asked }), { status: 201, headers: { "content-type": "application/json" } });
      }),
    );
    renderAt(BEHAVIOUR_PATH);
    const aside = await folders();
    fireEvent.click(chevron(aside, STORY) as HTMLElement);
    fireEvent.click(within(aside).getByText("Player signs up with an email"));
    fireEvent.click(choice(/^Only needing a grill pass\s*1$/));
    expect(pressed(/^Only needing a grill pass/)).toBe("true");

    const details = screen.getByRole("region", { name: "Details" });
    fireEvent.change(await within(details).findByRole("textbox", { name: "Add a question or remark" }), { target: { value: "Easy to miss." } });
    fireEvent.click(within(details).getByRole("button", { name: "Add to this node" }));

    await waitFor(() => expect(facts()).not.toContain("grill pass"));
    expect(pressed(/^Only needing a grill pass\s*0$/)).toBe("true");
  });
});
