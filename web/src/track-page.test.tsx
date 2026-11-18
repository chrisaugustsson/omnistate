// @vitest-environment jsdom
// A track's header (navigation F, tasks/ui-redesign/build-f.md): the title,
// then "···" and the primary action, the first state move the reader may make;
// the menu holds the other moves, Hand over and when the track was created.
// Only the moves the API allows (planning → building | dropped, building →
// planning | done | dropped), only for the owner or an admin, and done only for
// the owner; others see them off with the reason. A final move (dropped, done)
// asks first. Under the title one line of facts, each a link where it has a
// target: the state, the features it changes, the owner (Hand over), the tracks
// it was split from and off to, and the tracks whose stories block its own.
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

/** Waits until the members have loaded, so the header shows what the reader may do; gives the facts line. */
async function ready() {
  const facts = await screen.findByLabelText("Facts");
  await waitFor(() => expect(facts.textContent).toMatch(/owner (Alice|Bob|Cara)/));
  return facts;
}

/** The facts line as a person reads it. */
const factsText = (facts: HTMLElement) => facts.textContent?.replaceAll("·", " · ").replace(/\s+/g, " ").trim();

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
    const facts = await ready();
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
    // The owner in the facts is no button for them: they cannot hand the track over.
    expect(within(facts).queryByRole("button")).toBeNull();
    expect(within(facts).getByText("Bob").getAttribute("title")).toBe("Only Bob or an admin hands it over");
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
  test("the owner clicks their name in the facts, picks another member by name in the dialog, and the track goes to that membership", async () => {
    const api = fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows")] });
    renderAt(TRACK_PATH);
    const facts = await ready();
    fireEvent.click(within(facts).getByRole("button", { name: "Owner Alice: hand over" }));
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

describe("a track's facts line", () => {
  test("says its state, links the features it changes and the tracks it was split from and off to, and names its owner", async () => {
    fakeTrackApi({
      tracks: [
        track(TRACK_ID, "Long windows", { state: "building", split_from_track_id: SOURCE_ID }),
        track(SOURCE_ID, "Readings"),
        track(SPLIT_ID, "Aggregated readings", { part_id: IOT, split_from_track_id: TRACK_ID, owner_user_id: CARA }),
      ],
      trackFeatures: { [TRACK_ID]: [featureBody(ANALYZE, "Analyze")] },
    });
    renderAt(TRACK_PATH);
    const facts = await ready();
    await within(facts).findByRole("link", { name: "Analyze" });
    expect(factsText(facts)).toBe("Building · changes Analyze · owner Alice · split off from Readings · split off to Aggregated readings");
    expect(within(facts).getByRole("link", { name: "Analyze" }).getAttribute("href")).toBe(`${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`);
    expect(within(facts).getByRole("link", { name: "Readings" }).getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SOURCE_ID}`);
    expect(within(facts).getByRole("link", { name: "Aggregated readings" }).getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SPLIT_ID}`);
  });

  test("names the track whose open story blocks one of its own, linking to that track's board, and drops it once that story is done", async () => {
    const blocking = (status: string) => ({
      id: "s-their",
      key: "US-3",
      title: "Readings per hour",
      track_id: SPLIT_ID,
      track_name: "Aggregated readings",
      part_id: IOT,
      part_name: "iot-service",
      status,
      relation: "blocks",
      story_id: "s-ours",
      story_key: "US-1",
      blocker_dropped: false,
    });
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows"), track(SPLIT_ID, "Aggregated readings", { part_id: IOT })], linked: { [TRACK_ID]: [blocking("review")] } });
    renderAt(TRACK_PATH);
    let facts = await ready();
    const blocker = await within(facts).findByRole("link", { name: "Aggregated readings" });
    expect(factsText(facts)).toBe("Planning · owner Alice · blocked by Aggregated readings");
    expect(blocker.getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SPLIT_ID}/board`);
    expect(blocker.getAttribute("title")).toBe("iot-service US-3 blocks our US-1");
    cleanup();

    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows"), track(SPLIT_ID, "Aggregated readings", { part_id: IOT })], linked: { [TRACK_ID]: [blocking("done")] } });
    renderAt(TRACK_PATH);
    facts = await ready();
    await waitFor(() => expect(factsText(facts)).toBe("Planning · owner Alice"));
  });

  test("the menu says when the track was created", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows", { created_at: "2025-03-04T10:00:00Z" })] });
    renderAt(TRACK_PATH);
    await ready();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    expect(within(await screen.findByRole("menu", { name: "Track actions" })).getByText("Created 4 Mar 2025")).toBeTruthy();
  });

  test("the header is the same line on every section, kept in place from Spec to Board and How; only Spec opens with the description", async () => {
    fakeTrackApi({ tracks: [track(TRACK_ID, "Long windows", { description: "Hours to months in Analyze." })] });
    const router = renderAt(TRACK_PATH);
    await ready();
    const line = screen.getByRole("heading", { level: 1, name: "Long windows" }).parentElement!;
    expect(within(line).getByLabelText("Facts")).toBeTruthy();
    expect(within(within(line).getByRole("group", { name: "Actions" })).getByRole("button", { name: "Start building" })).toBeTruthy();
    expect(screen.getByText("Hours to months in Analyze.")).toBeTruthy();

    for (const section of ["board", "how"]) {
      await router.navigate({ to: `${TRACK_PATH}/${section}` });
      await waitFor(() => expect(router.state.location.pathname).toBe(`${TRACK_PATH}/${section}`));
      expect(screen.getByRole("heading", { level: 1, name: "Long windows" }).parentElement).toBe(line);
      expect(screen.queryByText("Hours to months in Analyze.")).toBeNull();
    }
  });

  test("a track the project does not have says so", async () => {
    fakeTrackApi({ tracks: [track(SOURCE_ID, "Readings")], members: MEMBERS });
    renderAt(TRACK_PATH);
    expect(await screen.findByRole("heading", { level: 1, name: "Track not found" })).toBeTruthy();
  });
});
