// @vitest-environment jsdom
// US-221: a feature's Behaviour screen, .../features/<feature>/behaviour. It
// reads GET .../features/<feature>/behaviour and lists the tree as folders: the
// feature first, then its stories, closed, by their short titles. An empty list
// is a feature with no tree; any failed request, 404 included, is a failed load.
// The app is rendered with its own router on a memory history and the
// browser's fetch is replaced by a fake of the API.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

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

/**
 * A fake of the API whose behaviour list answers with `behaviour` and whose
 * home data holds `features`, or never arrives when `features` is "pending".
 */
function fakeApi(behaviour: Behaviour, features: unknown[] | "pending" = []) {
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
