// @vitest-environment jsdom
// A track's page (UI redesign, slice S3): the header's primary action is the
// first state move the reader may make, the "…" menu holds the other moves and
// Hand over; only the moves the API allows (planning → building | dropped,
// building → planning | done | dropped), only for the owner or an admin, and
// done only for the owner; others see them off with the reason. A final move
// (dropped, done) asks first. The properties panel shows the track's part,
// the features it changes, its split links, the stories of other tracks that
// block its own, and its progress.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import {
  ALICE,
  API,
  BOB,
  CARA,
  FakeWebSocket,
  IOT,
  MEMBERS,
  PROJECT_PATH,
  WISION,
  fakeTrackApi,
  featureBody,
  member,
  track,
} from "./testing/track-api";

const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
const SOURCE_ID = "0192d7a0-4444-7000-8000-000000000002";
const SPLIT_ID = "0192d7a0-4444-7000-8000-000000000003";
const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
const TRACK_PATH = `${PROJECT_PATH}/tracks/${TRACK_ID}`;

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The header's buttons once the reader's rights are known: the primary move and the menu's items, each with whether it is on. */
async function headerMoves() {
  const actions = await screen.findByRole("group", { name: "Actions" });
  const more = within(actions).getByRole("button", { name: "More actions" });
  const primary = within(actions).queryByRole("button", { name: /^(Start building|Mark done|Back to planning)$/ }) as HTMLButtonElement | null;
  fireEvent.click(more);
  const menu = await screen.findByRole("menu", { name: "Track actions" });
  const items = within(menu)
    .getAllByRole("menuitem")
    .map((item) => [item.textContent?.replace(/(Owner only|Owner or admin)$/, "").trim(), item.getAttribute("aria-disabled") !== "true"]);
  fireEvent.click(more);
  return { primary: primary ? [primary.textContent, !primary.disabled] : null, items };
}

/** Waits until the members have loaded, so the header shows what the reader may do. */
async function ready() {
  const panel = await screen.findByRole("complementary", { name: "Properties" });
  await waitFor(() => expect(within(panel).queryByText("Nobody")).toBeNull());
  await waitFor(() => expect(panel.textContent).toMatch(/Alice|Bob|Cara/));
  return panel;
}

const stateWrites = (api: ReturnType<typeof fakeTrackApi>) =>
  api.calls.filter((call) => call.method === "POST" && call.path === `${API}/tracks/${TRACK_ID}/state`).map((call) => call.body);

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the state moves in a track's header", () => {
  test("the owner of a planning track gets Start building as the primary action, which moves it to building, and Drop track in the menu", async () => {
    const api = fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")] });
    renderAt(TRACK_PATH);
    await ready();
    expect(await headerMoves()).toEqual({
      primary: ["Start building", true],
      items: [
        ["Hand over…", true],
        ["Copy link", true],
        ["Drop track…", true],
      ],
    });
    fireEvent.click(within(screen.getByRole("group", { name: "Actions" })).getByRole("button", { name: "Start building" }));
    await waitFor(() => expect(stateWrites(api)).toEqual([{ state: "building" }]));
  });

  test("the owner of a building track gets Mark done first, and Back to planning and Drop track in the menu", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows", { state: "building" })] });
    renderAt(TRACK_PATH);
    await ready();
    expect(await headerMoves()).toEqual({
      primary: ["Mark done", true],
      items: [
        ["Hand over…", true],
        ["Copy link", true],
        ["Back to planning", true],
        ["Drop track…", true],
      ],
    });
  });

  test("an admin who does not own a building track may make every move but done, which is off", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows", { state: "building", owner_user_id: BOB })],
      members: [member("m-alice", ALICE.id, ALICE.email, "admin"), member("m-bob", BOB, "bob@example.com")],
    });
    renderAt(TRACK_PATH);
    await ready();
    expect(await headerMoves()).toEqual({
      primary: ["Back to planning", true],
      items: [
        ["Hand over…", true],
        ["Copy link", true],
        ["Mark done", false],
        ["Drop track…", true],
      ],
    });
  });

  test("a member who neither owns the track nor is an admin sees the moves and Hand over off, and why", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows", { owner_user_id: BOB })] });
    renderAt(TRACK_PATH);
    const panel = await ready();
    expect(await headerMoves()).toEqual({
      primary: ["Start building", false],
      items: [
        ["Hand over…", false],
        ["Copy link", true],
        ["Drop track…", false],
      ],
    });
    expect(within(screen.getByRole("group", { name: "Actions" })).getByRole("button", { name: "Start building" }).getAttribute("title")).toBe(
      "Only Bob or an admin moves this track",
    );
    expect(panel.textContent).toContain("Only Bob or an admin hands it over");
  });

  test("a done or a dropped track offers no move", async () => {
    for (const state of ["done", "dropped"]) {
      fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows", { state })] });
      renderAt(TRACK_PATH);
      await ready();
      const { primary, items } = await headerMoves();
      expect(primary, state).toBeNull();
      expect(items.map(([label]) => label), state).toEqual(["Hand over…", "Copy link"]);
      cleanup();
    }
  });

  test("Drop track asks first: Cancel moves nothing, confirming drops it", async () => {
    const api = fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")] });
    renderAt(TRACK_PATH);
    await ready();
    const drop = async () => {
      fireEvent.click(screen.getByRole("button", { name: "More actions" }));
      fireEvent.click(within(await screen.findByRole("menu")).getByRole("menuitem", { name: "Drop track…" }));
      return screen.findByRole("dialog", { name: "Drop Long windows?" });
    };
    fireEvent.click(within(await drop()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(stateWrites(api)).toEqual([]);

    fireEvent.click(within(await drop()).getByRole("button", { name: "Drop track" }));
    await waitFor(() => expect(stateWrites(api)).toEqual([{ state: "dropped" }]));
  });

  test("a move the API refuses shows its error under the title", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")], refuse: { status: 422, error: "invalid_transition" } });
    renderAt(TRACK_PATH);
    await ready();
    fireEvent.click(within(await screen.findByRole("group", { name: "Actions" })).getByRole("button", { name: "Start building" }));
    expect((await screen.findByRole("alert")).textContent).toBe("A track in planning cannot move to building.");
  });
});

describe("handing a track over", () => {
  test("the owner picks another member by name in the dialog, and the track goes to that membership", async () => {
    const api = fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")] });
    renderAt(TRACK_PATH);
    const panel = await ready();
    fireEvent.click(within(panel).getByRole("button", { name: "Hand over" }));
    const dialog = await screen.findByRole("dialog", { name: "Hand over" });
    const group = within(dialog).getByRole("radiogroup", { name: "Hand over to" });
    expect(within(group).getAllByRole("radio")).toEqual([
      within(group).getByRole("radio", { name: "Bob" }),
      within(group).getByRole("radio", { name: "Cara" }),
    ]);
    fireEvent.click(within(group).getByRole("radio", { name: "Cara" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Hand over" }));
    await waitFor(() =>
      expect(api.calls.filter((call) => call.method === "POST")).toEqual([
        { method: "POST", path: `${API}/tracks/${TRACK_ID}/owner`, body: { membership_id: "m-cara" } },
      ]),
    );
  });
});

describe("a track's properties panel", () => {
  test("shows its part, the features it changes and the tracks it was split from and off to, each linking to its page, and its progress", async () => {
    fakeTrackApi({
      tracks: [
        track(TRACK_ID, "Long windows", { state: "building", split_from_track_id: SOURCE_ID }),
        track(SOURCE_ID, "Readings"),
        track(SPLIT_ID, "Aggregated readings", { part_id: IOT, split_from_track_id: TRACK_ID, owner_user_id: CARA }),
      ],
      stories: { [TRACK_ID]: { done: 1, total: 3 } },
      trackFeatures: { [TRACK_ID]: [featureBody(ANALYZE, "Analyze")] },
    });
    renderAt(TRACK_PATH);
    const panel = await ready();
    expect(within(panel).getByRole("link", { name: "Wision" }).getAttribute("href")).toBe(`${PROJECT_PATH}/parts/${WISION}`);
    expect((await within(panel).findByRole("link", { name: "Analyze" })).getAttribute("href")).toBe(
      `${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`,
    );
    expect(panel.textContent).toContain("1 of 3 stories");
    const from = within(panel).getByRole("link", { name: /Readings.*Split from/ });
    expect(from.getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SOURCE_ID}`);
    const to = within(panel).getByRole("link", { name: /Aggregated readings.*Split off to/ });
    expect(to.getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SPLIT_ID}`);
    expect(to.textContent).toContain("iot-service · Cara");
  });

  test("names a story of another track that blocks one of its own, with both keys, linking to that track's board", async () => {
    fakeTrackApi({
      tracks: [track(TRACK_ID, "Long windows"), track(SPLIT_ID, "Aggregated readings", { part_id: IOT })],
      linked: {
        [TRACK_ID]: [
          {
            id: "s-their",
            key: "US-3",
            title: "Readings per hour",
            track_id: SPLIT_ID,
            track_name: "Aggregated readings",
            part_id: IOT,
            part_name: "iot-service",
            status: "review",
            relation: "blocks",
            story_id: "s-ours",
            story_key: "US-1",
            blocker_dropped: false,
          },
        ],
      },
    });
    renderAt(TRACK_PATH);
    const panel = await ready();
    const blocker = await within(panel).findByRole("link", { name: /Blocked by iot-service US-3/ });
    expect(blocker.textContent).toContain("Our US-1 waits · theirs is in review");
    expect(blocker.getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SPLIT_ID}/board`);
  });

  test("a track the project does not have says so", async () => {
    fakeTrackApi({ tracks: [track(SOURCE_ID, "Readings")], members: MEMBERS });
    renderAt(TRACK_PATH);
    expect(await screen.findByRole("heading", { level: 1, name: "Track not found" })).toBeTruthy();
  });
});
