// @vitest-environment jsdom
// A feature's pages (UI redesign, slice S3; prototype
// tasks/ui-redesign/v2/feature-analyze*.html): the Overview says what it does
// now, a banner names each track in flight that changes it; the tabs are
// Overview, Docs, Design (only with design docs), Tracks and Decisions; Tracks
// groups the tracks that changed it by state; Start track opens a new track on
// the feature's part that changes it.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { API, FakeWebSocket, PROJECT_PATH, WISION, docBody, fakeTrackApi, featureBody, track } from "./testing/track-api";

const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
const WINDOWS = "0192d7a0-4444-7000-8000-000000000001";
const CHART = "0192d7a0-4444-7000-8000-000000000002";
const FEATURE_PATH = `${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`;

const analyze = featureBody(ANALYZE, "Analyze", { description: "Time-series chart per channel, compare devices, radar view, export." });

function featureApi(fields: { docs?: ReturnType<typeof docBody>[]; feature?: ReturnType<typeof featureBody> } = {}) {
  return fakeTrackApi({
    tracks: [
      track(WINDOWS, "Long time windows", { description: "Adds 90 d and 1 y windows. Readings aggregate per hour." }),
      track(CHART, "First chart", { state: "done" }),
    ],
    stories: { [WINDOWS]: { done: 0, total: 2 } },
    features: [fields.feature ?? analyze],
    featureTracks: { [ANALYZE]: [WINDOWS, CHART] },
    docs: fields.docs,
  });
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

async function tabs() {
  const nav = await screen.findByRole("navigation", { name: "Feature" });
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

describe("a feature's Overview", () => {
  test("says what it does now, and a banner names the track in flight that changes it, linking to it; a done track has none", async () => {
    featureApi();
    renderAt(FEATURE_PATH);
    const now = (await screen.findByRole("heading", { name: "What it does now" })).closest("section") as HTMLElement;
    expect(now.textContent).toContain("Time-series chart per channel, compare devices, radar view, export.");
    const banner = await screen.findByRole("link", { name: /Being changed by/ });
    expect(banner.textContent).toContain("Being changed by Long time windows: Adds 90 d and 1 y windows. Not on main yet.");
    expect(banner.getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${WINDOWS}`);
    expect(screen.getAllByRole("link", { name: /Being changed by/ })).toHaveLength(1);
  });

  test("with nothing written it offers Add a description; the text saved is what it does now", async () => {
    const api = featureApi({ feature: featureBody(ANALYZE, "Analyze") });
    renderAt(FEATURE_PATH);
    const now = (await screen.findByRole("heading", { name: "What it does now" })).closest("section") as HTMLElement;
    fireEvent.click(await within(now).findByRole("button", { name: "Add a description" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Description" }), { target: { value: "Charts each channel over time." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(api.calls.filter((call) => call.method !== "GET")).toEqual([
        { method: "PATCH", path: `${API}/parts/${WISION}/features/${ANALYZE}`, body: { description: "Charts each channel over time." } },
      ]),
    );
    expect(await within(now).findByText("Charts each channel over time.")).toBeTruthy();
    expect(within(now).queryByRole("button", { name: "Add a description" })).toBeNull();
  });
});

describe("a feature's tabs", () => {
  test("leave out Design while the feature has no design doc", async () => {
    featureApi({ docs: [docBody("0192d7a0-6666-7000-8000-000000000001", "How compare works", { pillar: "architecture", feature_id: ANALYZE })] });
    renderAt(FEATURE_PATH);
    await waitFor(async () => expect(await tabs()).toEqual(["Overview", "Docs1", "Tracks2", "Decisions"]));
  });

  test("show Design once the feature has a design doc", async () => {
    featureApi({ docs: [docBody("0192d7a0-6666-7000-8000-000000000002", "Analyze screens", { pillar: "ui", kind: "design", feature_id: ANALYZE })] });
    renderAt(FEATURE_PATH);
    await waitFor(async () => expect(await tabs()).toEqual(["Overview", "Docs1", "Design1", "Tracks2", "Decisions"]));
  });
});

describe("a feature's Design", () => {
  test("the design's header folds the doc list away for the canvas and brings it back", async () => {
    localStorage.removeItem("omni:pane:doc-list");
    try {
      featureApi({ docs: [docBody("0192d7a0-6666-7000-8000-000000000002", "Analyze screens", { pillar: "ui", kind: "design", feature_id: ANALYZE })] });
      renderAt(`${FEATURE_PATH}/design`);
      const list = () => screen.queryByRole("navigation", { name: "Doc list" });
      await screen.findByRole("heading", { name: "Analyze screens" });
      expect(list()).toBeTruthy();

      fireEvent.click(screen.getByRole("button", { name: "Hide the doc list" }));
      expect(list()).toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "Show the doc list" }));
      expect(list()).toBeTruthy();
    } finally {
      localStorage.removeItem("omni:pane:doc-list");
    }
  });
});

describe("a feature's Tracks", () => {
  test("groups the tracks that changed it by state", async () => {
    featureApi();
    renderAt(`${FEATURE_PATH}/tracks`);
    const planning = await screen.findByRole("region", { name: "Planning" });
    expect(within(planning).getAllByRole("link").map((link) => link.getAttribute("href"))).toEqual([`${PROJECT_PATH}/tracks/${WINDOWS}`]);
    const done = screen.getByRole("region", { name: "Done" });
    expect(within(done).getAllByRole("link").map((link) => link.getAttribute("href"))).toEqual([`${PROJECT_PATH}/tracks/${CHART}`]);
    expect(screen.queryByRole("region", { name: "Building" })).toBeNull();
  });
});

describe("Start track", () => {
  test("creates a track on the feature's part, links it to the feature, and opens it", async () => {
    const api = featureApi();
    const router = renderAt(FEATURE_PATH);
    fireEvent.click(await screen.findByRole("button", { name: "Start track" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Track name"), { target: { value: "Radar export" } });
    fireEvent.change(within(dialog).getByLabelText("Description"), { target: { value: "Export the radar view." } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Start track" }));

    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/tracks/0192d7a0-4444-7000-8000-0000000000ff`));
    expect(api.calls.filter((call) => call.method === "POST").map(({ path, body }) => ({ path, body }))).toEqual([
      { path: `${API}/tracks`, body: { name: "Radar export", description: "Export the radar view.", part_id: WISION } },
      { path: `${API}/tracks/0192d7a0-4444-7000-8000-0000000000ff/features`, body: { feature_id: ANALYZE } },
    ]);
  });
});
