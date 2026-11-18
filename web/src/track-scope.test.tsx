// @vitest-environment jsdom
// A track's sections, in the shell's sidebar (navigation F): Spec, Behaviour,
// Design, How, Board, Threads, Decisions, with counts: open behaviour questions an agent asked (a
// person's remark does not wait), the design docs' boards, stories done of
// total, threads and decisions. A track on a service part has no Design tab
// unless it has design docs. While questions wait a quiet Next row leads to
// them. Spec, Design and How each show the track's doc of that pillar (How:
// architecture and tables), or say which agent writes it.
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

describe("the Next row", () => {
  test("while agents' questions wait on a planning track, it says how many and leads to Behaviour", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows")],
      questions: { [TRACK_ID]: [question("q1", "agent"), question("q2", "agent"), question("q3", "human")] },
    });
    renderAt(TRACK_PATH);
    const next = (await screen.findByText(/^Next:/)).closest("div") as HTMLElement;
    expect(next.textContent).toContain("Next: answer 2 behaviour questions, then start building.");
    expect(within(next).getByRole("link").getAttribute("href")).toBe(`${TRACK_PATH}/behaviour`);
  });

  test("is not on the Behaviour tab, where it leads, and neither is the properties toggle", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows")],
      questions: { [TRACK_ID]: [question("q1", "agent"), question("q2", "agent")] },
    });
    renderAt(`${TRACK_PATH}/behaviour`);
    const actions = await screen.findByRole("group", { name: "Actions" });
    await waitFor(() => expect(screen.getByRole("link", { name: /Behaviour/ }).textContent).toContain("2"));
    expect(screen.queryByText(/^Next:/)).toBeNull();
    expect(within(actions).queryByRole("button", { name: "Properties" })).toBeNull();
    expect(screen.queryByRole("complementary", { name: "Properties" })).toBeNull();
  });

  test("is not there when only a person's remark is open", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")], questions: { [TRACK_ID]: [question("q3", "human")] } });
    renderAt(TRACK_PATH);
    await screen.findByRole("complementary", { name: "Properties" });
    await waitFor(async () => expect(await tabs()).toContain("Behaviour"));
    expect(screen.queryByText(/^Next:/)).toBeNull();
  });
});

describe("the properties toggle", () => {
  test("is pressed while the panel shows and not once it hides it", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")] });
    renderAt(TRACK_PATH);
    await screen.findByRole("complementary", { name: "Properties" });
    const toggle = within(screen.getByRole("group", { name: "Actions" })).getByRole("button", { name: "Properties" });
    expect(toggle.getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(toggle);
    await waitFor(() => expect(screen.queryByRole("complementary", { name: "Properties" })).toBeNull());
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
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
