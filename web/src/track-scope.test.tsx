// @vitest-environment jsdom
// A track's sections, in the shell's sidebar (navigation F): Spec, Behaviour,
// Design, How, Board, Threads, Decisions, with counts: open behaviour questions an agent asked (a
// person's remark does not wait), the design docs' boards, stories done of
// total, threads and decisions. A track on a service part has no Design tab
// unless it has design docs. Spec, Design and How each show the track's doc of
// that pillar (How: architecture and tables), or say which agent writes it. On
// the phone the Behaviour is an outline of the tree, a node opening full screen.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { ALICE, FakeWebSocket, IOT, PROJECT_PATH, docBody, fakeTrackApi, track } from "./testing/track-api";

const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
const TRACK_PATH = `${PROJECT_PATH}/tracks/${TRACK_ID}`;
const SPEC = "0192d7a0-6666-7000-8000-000000000001";
const OPS = "0192d7a0-6666-7000-8000-000000000002";
const BOARDS = "0192d7a0-6666-7000-8000-000000000003";

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

const question = (id: string, kind: "agent" | "human") => ({
  id,
  node_id: "n1",
  node_path: "",
  question: "Which window?",
  state: "open",
  asked_by: kind === "agent" ? { kind: "agent", role: "behaviour" } : { kind: "human", user_id: ALICE.id },
  answers: [],
  created_at: "2026-11-10T08:00:00Z",
  version: 1,
});

const thread = (id: string) => ({ id, project_id: "p", track_id: TRACK_ID, question: `Thread ${id}?`, created_at: "2026-11-10T08:00:00Z", post_count: 0 });

const decision = (id: string) => ({
  id,
  project_id: "p",
  track_id: TRACK_ID,
  about: "track",
  question: "Q?",
  decision: `Decision ${id}`,
  why: "Because",
  author: { kind: "agent", role: "grill" },
  created_at: "2026-11-10T08:00:00Z",
});

/** The track's sections in the sidebar as their text: name and count. */
async function tabs() {
  const nav = await screen.findByRole("navigation", { name: "Sections" });
  return within(nav)
    .getAllByRole("link")
    .map((link) => link.textContent);
}

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("a track's sections", () => {
  test("count the agents' open questions, the design boards, the stories done of total, the threads and the decisions", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows")],
      stories: { [TRACK_ID]: { done: 1, total: 3 } },
      questions: { [TRACK_ID]: [question("q1", "agent"), question("q2", "agent"), question("q3", "human")] },
      docs: [docBody(BOARDS, "Window picker", { pillar: "ui", kind: "design", track_id: TRACK_ID })],
      boards: { [BOARDS]: ["b1", "b2", "b3", "b4"] },
      threads: [thread("t1"), thread("t2")],
      trackDecisions: { [TRACK_ID]: [decision("d1"), decision("d2"), decision("d3")] },
    });
    renderAt(TRACK_PATH);
    await waitFor(async () =>
      expect(await tabs()).toEqual(["Spec", "Behaviour2", "Design4", "How", "Board1/3", "Threads2", "Decisions3"]),
    );
  });

  test("a track on a service part has no Design tab until it has a design doc", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Readings query", { part_id: IOT })] });
    renderAt(TRACK_PATH);
    await waitFor(async () => expect(await tabs()).toEqual(["Spec", "Behaviour", "How", "Board0/0", "Threads", "Decisions"]));
    cleanup();

    fakeTrackApi({
      tracks: [track(TRACK_ID, "Readings query", { part_id: IOT })],
      docs: [docBody(BOARDS, "Status page", { pillar: "ui", kind: "html", track_id: TRACK_ID })],
    });
    renderAt(TRACK_PATH);
    await waitFor(async () => expect(await tabs()).toContain("Design"));
  });
});

describe("Behaviour on the phone", () => {
  const node = (id: string, parentId: string | null, title: string, status = "answered") => ({
    id,
    parent_id: parentId,
    path: "",
    slug: null,
    position: 0,
    level: 0,
    title,
    summary: "",
    status,
    version: 1,
    open_questions: 0,
    default_answers: 0,
    needs_pass: false,
    same_as: null,
    handoff: null,
  });

  test("is the tree as an outline with its stories open and the questions waiting counted, and a node opens over the whole screen", async () => {
    vi.stubGlobal("matchMedia", (media: string) => ({ media, matches: true, addEventListener() {}, removeEventListener() {} }));
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows")],
      questions: { [TRACK_ID]: [{ ...question("q1", "agent"), node_id: "n3" }] },
      tree: { [TRACK_ID]: [node("n1", null, "Long windows"), node("n2", "n1", "Pick a window"), node("n3", "n2", "Picks one year", "open")] },
    });
    renderAt(`${TRACK_PATH}/behaviour`);
    const outline = await screen.findByRole("region", { name: "Behaviour outline" });
    await waitFor(() => expect(outline.textContent).toContain("1 question waits"));
    expect(screen.queryByRole("main", { name: "Tree as a graph" })).toBeNull();
    fireEvent.click(within(outline).getByText("Picks one year"));
    expect(await screen.findByRole("dialog", { name: "Picks one year" })).toBeTruthy();
    fireEvent.keyDown(document, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("the doc tabs", () => {
  const docs = [
    docBody(SPEC, "Spec", { pillar: "spec", track_id: TRACK_ID }),
    docBody(OPS, "Operations", { pillar: "architecture", track_id: TRACK_ID }),
  ];

  test("Spec shows the track's spec and How its architecture doc, each with a link to it in full view", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")], docs });
    renderAt(TRACK_PATH);
    expect((await screen.findByRole("link", { name: "Full view" })).getAttribute("href")).toBe(`${TRACK_PATH}/docs/${SPEC}`);
    cleanup();

    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")], docs });
    renderAt(`${TRACK_PATH}/how`);
    expect((await screen.findByRole("link", { name: "Full view" })).getAttribute("href")).toBe(`${TRACK_PATH}/docs/${OPS}`);
  });

  test("a tab whose pillar has no doc says which agent writes it", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")], docs: [docs[1]] });
    renderAt(TRACK_PATH);
    expect(await screen.findByText("No spec yet")).toBeTruthy();
    expect(screen.getByText(/grill agent writes the track's spec/)).toBeTruthy();
    expect(screen.queryByRole("link", { name: "Full view" })).toBeNull();
  });
});
