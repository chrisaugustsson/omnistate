// @vitest-environment jsdom
// US-138: the board of a feature, .../projects/<project>/features/<feature>/board.
// It reads GET .../features/<feature>/stories and shows a column per status and
// a card per story, filtered by priority and, unless Show cancelled is on,
// without the cancelled stories. The app is rendered with its own router on a
// memory history, the browser's fetch is replaced by a fake of the API, and the
// browser's WebSocket by a fake socket that the test drives.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const FEATURE_ID = "0192d7a0-4444-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const BOARD_PATH = `${PROJECT_PATH}/features/${FEATURE_ID}/board`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T09:00:00Z";
const FIVE_COLUMNS = ["Backlog", "Implementing", "Review", "Testing", "Done"];

type Criterion = { text: string; ticked: boolean; pending: boolean; human_check: boolean };
type Finding = { stage: string; text: string; files: string[]; resolved: boolean };

/** Story US-<number> of the feature; the ID is derived from the number. */
function story(
  number: number,
  fields: {
    status?: string;
    priority?: number;
    title?: string;
    holder?: { run: string; stage: string } | null;
    blocked?: boolean;
    blockedBy?: number[];
    findings?: Finding[];
    criteria?: Criterion[];
  } = {},
) {
  return {
    id: storyId(number),
    project_id: PROJECT_ID,
    feature_id: FEATURE_ID,
    number,
    key: `US-${number}`,
    title: fields.title ?? `Story ${number}`,
    text: "",
    topic: "",
    status: fields.status ?? "backlog",
    priority: fields.priority ?? 3,
    blocked: fields.blocked ?? false,
    blocked_by: (fields.blockedBy ?? []).map(storyId),
    criteria: fields.criteria ?? [],
    findings: fields.findings ?? [],
    decisions: [],
    files_planned: [],
    files_changed: [],
    links: [],
    holder: fields.holder ?? null,
    version: 1,
    created_at: AT,
    updated_at: AT,
  };
}

function storyId(number: number) {
  return `0192d7a0-7777-7000-8000-${number.toString(16).padStart(12, "0")}`;
}

function finding(resolved: boolean): Finding {
  return { stage: "review", text: "Handle the empty cart.", files: [], resolved };
}

function criterion(ticked: boolean, pending: boolean): Criterion {
  return { text: "The total is shown.", ticked, pending, human_check: pending };
}

type Story = ReturnType<typeof story>;

/** A fake of the API whose `stories` the test may replace before a live event. */
function fakeApi(stories: Story[]) {
  const api = { stories };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      switch (new URL(url, "http://localhost:3000").pathname) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
        case `${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            features: [],
            docs: [],
            threads: [],
            waiting: { comments: [] },
          });
        case `${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        case `${API}/features/${FEATURE_ID}/stories`:
          return json(200, { stories: api.stories });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
  return api;
}

/** A WebSocket the test drives: it records what the client sends. */
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
    if (this.readyState !== FakeWebSocket.OPEN) {
      throw new Error("InvalidStateError: the socket is not open");
    }
    this.sent.push(JSON.parse(data));
  }

  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.fire("close", { code: 1000, reason: "", wasClean: true });
  }

  /** The server accepted the connection. */
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.fire("open", {});
  }

  /** The server sent a JSON text frame. */
  receive(message: unknown) {
    this.fire("message", { data: JSON.stringify(message) });
  }

  private fire(type: string, fields: Record<string, unknown>) {
    const event = Object.assign(new Event(type), fields);
    const handler = (this as unknown as Record<string, unknown>)[`on${type}`];
    if (typeof handler === "function") handler.call(this, event);
    this.dispatchEvent(event);
  }
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The board, once its stories are loaded. */
async function board() {
  const main = await screen.findByRole("main");
  await within(main).findByRole("region", { name: "Backlog" });
  return main;
}

/** The titles of the board's columns, in order. */
function columnTitles(main: HTMLElement) {
  return Array.from(main.querySelectorAll("section")).map((section) => section.getAttribute("aria-label"));
}

function column(main: HTMLElement, title: string) {
  return within(main).getByRole("region", { name: title });
}

/** The keys of the cards in the column `title`, top to bottom. */
function cardKeys(main: HTMLElement, title: string) {
  return Array.from(column(main, title).querySelectorAll(":scope > ul > li")).map(
    (card) => card.querySelector("span")?.textContent,
  );
}

/** The card of the story `key`, wherever it sits. */
function card(main: HTMLElement, key: string) {
  const found = Array.from(main.querySelectorAll("section > ul > li")).find(
    (candidate) => candidate.querySelector("span")?.textContent === key,
  );
  expect(found, `a card for ${key}`).toBeTruthy();
  return found as HTMLElement;
}

/** The blocker `key` as the card `of` lists it. */
function blocker(main: HTMLElement, of: string, key: string) {
  return within(card(main, of)).getByText(key);
}

const choosePriority = (main: HTMLElement, value: string) =>
  fireEvent.change(within(main).getByRole("combobox", { name: /Priority/ }), { target: { value } });
const showCancelled = (main: HTMLElement) =>
  fireEvent.click(within(main).getByRole("checkbox", { name: "Show cancelled" }));

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the board", () => {
  test("a story sits only in its status's column, its card showing key, title, priority, holder, unresolved findings and its blockers, closed ones checked", async () => {
    fakeApi([
      story(131, { status: "done" }),
      story(132, {
        status: "review",
        priority: 2,
        title: "Pay with a saved card",
        holder: { run: "r1", stage: "review" },
        findings: [finding(false), finding(true)],
        blockedBy: [131, 133],
      }),
      story(133),
    ]);
    renderAt(BOARD_PATH);
    const main = await board();

    expect(columnTitles(main)).toEqual(FIVE_COLUMNS);
    expect(cardKeys(main, "Review")).toEqual(["US-132"]);
    expect(within(main).getAllByText("US-132"), "nowhere but under Review").toHaveLength(1);
    const shown = card(main, "US-132");
    expect(within(shown).getByText("Pay with a saved card")).toBeTruthy();
    expect(within(shown).getByText("P2")).toBeTruthy();
    expect(within(shown).getByText("review · r1")).toBeTruthy();
    expect(within(shown).getByText("1 findings")).toBeTruthy();
    expect(within(blocker(main, "US-132", "US-131")).getByLabelText("done")).toBeTruthy();
    expect(within(blocker(main, "US-132", "US-133")).queryByLabelText("done"), "an open blocker has no check mark").toBeNull();
    expect(within(card(main, "US-131")).queryByText(/findings/), "no badge at 0 findings").toBeNull();
  });

  test("a card counts the criteria ticked and pending a human check, hides the count at 0, and checks a cancelled blocker", async () => {
    fakeApi([
      story(1, {
        status: "testing",
        criteria: [criterion(true, true), criterion(true, false), criterion(false, false)],
        blockedBy: [2],
      }),
      story(2, { status: "cancelled" }),
      story(3, { status: "testing", criteria: [criterion(true, false)] }),
    ]);
    renderAt(BOARD_PATH);
    const main = await board();

    expect(within(card(main, "US-1")).getByText("1 to check")).toBeTruthy();
    expect(within(blocker(main, "US-1", "US-2")).getByLabelText("done")).toBeTruthy();
    expect(within(card(main, "US-3")).queryByText(/to check/)).toBeNull();
  });

  test("a blocked story shows Blocked in whatever column it sits, and an unblocked one does not", async () => {
    fakeApi([
      story(1, { status: "backlog", blocked: true }),
      story(2, { status: "done", blocked: true }),
      story(3, { status: "backlog" }),
    ]);
    renderAt(BOARD_PATH);
    const main = await board();

    expect(within(card(main, "US-1")).getByText("Blocked")).toBeTruthy();
    expect(within(card(main, "US-2")).getByText("Blocked")).toBeTruthy();
    expect(within(card(main, "US-3")).queryByText("Blocked")).toBeNull();
  });

  test("a cancelled story is hidden until Show cancelled adds a sixth column Cancelled after Done", async () => {
    fakeApi([story(1), story(2, { status: "cancelled" })]);
    renderAt(BOARD_PATH);
    const main = await board();

    expect(columnTitles(main)).toEqual(FIVE_COLUMNS);
    expect(within(main).queryByText("US-2")).toBeNull();

    showCancelled(main);
    expect(columnTitles(main)).toEqual([...FIVE_COLUMNS, "Cancelled"]);
    expect(cardKeys(main, "Cancelled")).toEqual(["US-2"]);
    expect(cardKeys(main, "Backlog")).toEqual(["US-1"]);
  });

  test("the priority filter applies to the Cancelled column too", async () => {
    fakeApi([story(1, { status: "cancelled", priority: 1 }), story(2, { status: "cancelled", priority: 3 })]);
    renderAt(BOARD_PATH);
    const main = await board();

    showCancelled(main);
    choosePriority(main, "1");
    expect(cardKeys(main, "Cancelled")).toEqual(["US-1"]);
  });

  test("Priority offers all, 1, 2 and 3; all, the default, shows a priority 0 card and 1 hides every other priority", async () => {
    fakeApi([
      story(1, { priority: 0 }),
      story(2, { priority: 1 }),
      story(3, { status: "review", priority: 1 }),
      story(4, { priority: 2 }),
      story(5, { priority: 3 }),
    ]);
    renderAt(BOARD_PATH);
    const main = await board();

    const options = within(within(main).getByRole("combobox", { name: /Priority/ })).getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual(["all", "1", "2", "3"]);
    expect(cardKeys(main, "Backlog")).toEqual(["US-1", "US-2", "US-4", "US-5"]);

    choosePriority(main, "1");
    expect(cardKeys(main, "Backlog")).toEqual(["US-2"]);
    expect(cardKeys(main, "Review")).toEqual(["US-3"]);
  });

  test("cards in a column come by priority, then number, whatever order the list gives", async () => {
    fakeApi([
      story(5, { priority: 3 }),
      story(2, { priority: 3 }),
      story(9, { priority: 1 }),
      story(7, { priority: 2 }),
    ]);
    renderAt(BOARD_PATH);
    const main = await board();

    expect(cardKeys(main, "Backlog")).toEqual(["US-9", "US-7", "US-2", "US-5"]);
  });

  test("a story.changed event for a story moved from Implementing to Review moves its card without a reload", async () => {
    const api = fakeApi([story(1, { status: "implementing" })]);
    renderAt(BOARD_PATH);
    const main = await board();
    expect(cardKeys(main, "Implementing")).toEqual(["US-1"]);

    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() =>
      expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })),
    );
    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });

    api.stories = [story(1, { status: "review" })];
    socket.receive({
      type: "event",
      seq: 11,
      event: "story.changed",
      project_id: PROJECT_ID,
      at: AT,
      data: { story_id: storyId(1), key: "US-1", fields: ["status"] },
    });
    await waitFor(() => expect(cardKeys(main, "Review")).toEqual(["US-1"]));
    expect(cardKeys(main, "Implementing")).toEqual([]);
  });
});
