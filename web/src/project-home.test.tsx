// @vitest-environment jsdom
// The Overview of a project (/w/<ws>/projects/<p>) and of a part
// (.../parts/<part>), and the Docs tabs of both (build plan, slice S2;
// prototype tasks/ui-redesign/v2/project.html, part-wision.html, docs.html).
// An Overview shows the intro as plain page text (the project's description,
// editable in place, "Add a description" when empty: decision U8), the tracks
// in flight with their owners named from their email, a part's features with
// the track changing one, the newest decisions and docs, and the counts. A
// Docs tab lists the scope's docs grouped by pillar beside the open doc; with
// none open the newest opens.
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
  API,
  FakeWebSocket,
  PROJECT_ID,
  PROJECT_PATH,
  decision,
  doc,
  fakeScopeApi,
  feature,
  me,
  member,
  part,
  renderAt,
  shownText,
  track,
} from "./scope-pages-fake";

const main = () => screen.findByRole("main");
const region = async (name: string) => within(await main()).findByRole("region", { name });
/** The rows of a section's list, as the reader sees each. */
const rows = (container: HTMLElement) => within(container).getAllByRole("listitem").map(shownText);

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date("2026-11-17T12:00:00Z"));
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the project's Overview", () => {
  test("with no description it offers Add a description; the text saved is the intro, and Edit opens it again", async () => {
    const api = fakeScopeApi({ description: null });
    renderAt(PROJECT_PATH);
    fireEvent.click(await within(await main()).findByRole("button", { name: "Add a description" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Description" }), {
      target: { value: "Sensor monitoring for customers.\n\nAsk Sara about readings." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: API, body: { description: "Sensor monitoring for customers.\n\nAsk Sara about readings." } }]));
    const page = await main();
    expect(await within(page).findByText("Ask Sara about readings.")).toBeTruthy();
    expect(within(page).getByText("Sensor monitoring for customers.").tagName).toBe("P");
    expect(within(page).queryByRole("button", { name: "Add a description" })).toBeNull();

    fireEvent.click(within(page).getByRole("button", { name: "Edit the description" }));
    expect((screen.getByRole("textbox", { name: "Description" }) as HTMLTextAreaElement).value).toBe(
      "Sensor monitoring for customers.\n\nAsk Sara about readings.",
    );
  });

  test("In flight lists the planning and building tracks, the most recently active first, with what each changes, its part, progress and owner by name; ended ones are under Last finished", async () => {
    const sara = member("sara.lind@example.com");
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    const analyze = feature(wision.id, "Analyze");
    const windows = track("Long time windows", wision.id, "planning", { owner: me.user_id, total: 2, lastActivity: "2026-11-17T11:00:00Z" });
    const query = track("Aggregated readings", iot.id, "building", {
      owner: sara.user_id,
      done: 1,
      total: 2,
      lastActivity: "2026-11-17T11:30:00Z",
      splitFrom: windows.track.id,
    });
    const alarms = track("Alarm thresholds", wision.id, "done", { lastActivity: "2026-11-04T10:00:00Z" });
    const dashboards = track("Move dashboards", iot.id, "dropped", { lastActivity: "2026-11-16T10:00:00Z" });
    fakeScopeApi({
      members: [me, sara],
      parts: [wision, iot],
      features: [analyze],
      tracks: [windows, query, alarms, dashboards],
      changes: { [analyze.id]: [windows.track.id] },
    });
    renderAt(PROJECT_PATH);

    const flight = await region("In flight");
    await waitFor(() => expect(rows(flight)[1]).toContain("changes Analyze"));
    expect(rows(flight)).toEqual([
      "Aggregated readings changes no feature · split off from Long time windows iot-service 1 of 2 Owned by Sara",
      "Long time windows changes Analyze · split off to Aggregated readings Wision 0 of 2 Owned by Christopher",
    ]);
    expect(within(flight).getByRole("link", { name: /^PlanningLong time windows/ }).getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${windows.track.id}`);
    expect(flight.textContent).toContain("Last finished: Move dashboards, dropped 16 Nov · Alarm thresholds, done 4 Nov");
  });

  test("Recent decisions shows the newest three of every level as 3 of N, each with who made it and what it is about", async () => {
    const sara = member("sara.lind@example.com");
    const wision = part("Wision", "app");
    const analyze = feature(wision.id, "Analyze");
    const windows = track("Long time windows", wision.id, "planning");
    fakeScopeApi({
      members: [me, sara],
      parts: [wision],
      features: [analyze],
      tracks: [windows],
      decisions: [
        decision("Apps reach services only through api-gateway", windows.track.id, { project: true }, { at: "2026-11-11T08:00:00Z" }),
        decision("Presets only", windows.track.id, { feature: analyze, part: wision }, { by: sara, at: "2026-11-14T08:00:00Z" }),
        decision("Charts draw on canvas", windows.track.id, { part: wision }, { by: "how", at: "2026-11-16T08:00:00Z" }),
        decision("Settings per organisation", windows.track.id, { part: wision }, { at: "2026-11-15T08:00:00Z" }),
      ],
    });
    renderAt(PROJECT_PATH);

    const recent = await region("Recent decisions");
    await waitFor(() => expect(recent.textContent).toContain("3 of 4"));
    expect(rows(recent)).toEqual([
      "Charts draw on canvas How agent · about Wision · from Long time windows 16 Nov",
      "Settings per organisation Christopher · about Wision · from Long time windows 15 Nov",
      "Presets only Sara · about Wision › Analyze · from Long time windows 14 Nov",
    ]);
  });

  test("Recent docs are the three most recently updated docs of the project itself, not those of its parts, features or tracks", async () => {
    const wision = part("Wision", "app");
    const analyze = feature(wision.id, "Analyze");
    const windows = track("Long time windows", wision.id, "planning");
    const onboarding = doc("Onboarding", "spec", { updatedAt: "2026-11-16T08:00:00Z" });
    const overview = doc("Architecture overview", "architecture", { updatedAt: "2026-11-15T08:00:00Z" });
    const routes = doc("Gateway routes", "architecture", { updatedAt: "2026-11-14T08:00:00Z" });
    fakeScopeApi({
      parts: [wision],
      features: [analyze],
      tracks: [windows],
      docs: [
        doc("Old notes", "research", { updatedAt: "2026-10-01T08:00:00Z" }),
        routes,
        doc("Wision design system", "ui", { part: wision.id, updatedAt: "2026-11-17T08:00:00Z" }),
        doc("How Analyze works", "spec", { feature: analyze.id, updatedAt: "2026-11-17T09:00:00Z" }),
        doc("Track spec", "spec", { track: windows.track.id, updatedAt: "2026-11-17T10:00:00Z" }),
        overview,
        onboarding,
      ],
    });
    renderAt(PROJECT_PATH);

    const recent = await region("Recent docs");
    await within(recent).findByText("Onboarding");
    expect(recent.textContent).toContain("3 of 4");
    expect(within(recent).getAllByRole("link", { name: /^(?!All docs)/ }).map((link) => [shownText(link), link.getAttribute("href")])).toEqual([
      ["Onboarding Spec · 1 d ago", `${PROJECT_PATH}/docs/${onboarding.id}`],
      ["Architecture overview Architecture · 2 d ago", `${PROJECT_PATH}/docs/${overview.id}`],
      ["Gateway routes Architecture · 3 d ago", `${PROJECT_PATH}/docs/${routes.id}`],
    ]);
  });

  test("the panel counts parts, features, the project's own docs, tracks in flight and done, and decisions nobody replaced", async () => {
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    const windows = track("Long time windows", wision.id, "planning");
    const old = decision("Readings are always raw", windows.track.id, { project: true }, { at: "2026-11-01T08:00:00Z" });
    const now = decision("Aggregate per hour", windows.track.id, { project: true }, { replaces: old.id });
    old.replaced_by_decision_id = now.id;
    fakeScopeApi({
      parts: [wision, iot],
      features: [feature(wision.id, "Analyze"), feature(wision.id, "Alarms"), feature(iot.id, "Ingest")],
      tracks: [windows, track("Query", iot.id, "building"), track("Charts", wision.id, "done"), track("Moves", iot.id, "dropped")],
      docs: [doc("Onboarding", "spec"), doc("Tables", "db", { part: iot.id })],
      decisions: [old, now],
    });
    renderAt(PROJECT_PATH);

    const panel = within(await main()).getByRole("complementary", { name: "Properties" });
    await waitFor(() =>
      expect(within(panel).getAllByRole("link", { name: /^\d/ }).map(shownText)).toEqual([
        "2 parts",
        "3 features",
        "1 doc",
        "2 in flight",
        "1 done",
        "1 decision",
      ]),
    );
    expect(within(panel).getByText("1 app, 1 service")).toBeTruthy();
  });

  test("In flight updates as the project's live events come in", async () => {
    const wision = part("Wision", "app");
    const api = fakeScopeApi({ parts: [wision], tracks: [track("Long time windows", wision.id, "planning")] });
    renderAt(PROJECT_PATH);
    const flight = await region("In flight");
    await within(flight).findByText("Long time windows");

    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() => expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })));
    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 10 });
    api.world.tracks = [...api.world.tracks, track("Exports", wision.id, "building", { lastActivity: "2026-11-17T11:59:00Z" })];
    socket.receive({ type: "event", seq: 11, event: "track.created", project_id: PROJECT_ID, at: "2026-11-17T11:59:00Z", data: {} });

    const list = within(flight).getByRole("list", { name: "Tracks in flight" });
    await waitFor(() => expect(rows(list).map((row) => row.split(" changes")[0])).toEqual(["Exports", "Long time windows"]));
  });
});

describe("a part's Overview", () => {
  test("lists the part's features, the one a track in flight changes marked Being changed, and its own tracks in flight without a part", async () => {
    const wision = part("Wision", "app", { description: "The web app customers log in to." });
    const iot = part("iot-service", "service");
    const dashboards = feature(wision.id, "Dashboards", "Widgets for latest readings");
    const analyze = feature(wision.id, "Analyze", "Time-series chart per channel");
    const windows = track("Long time windows", wision.id, "building", { total: 2 });
    const charts = track("First chart", wision.id, "done", { total: 9, done: 9 });
    fakeScopeApi({
      parts: [wision, iot],
      features: [dashboards, analyze, feature(iot.id, "Ingest")],
      tracks: [windows, charts, track("Query", iot.id, "building")],
      changes: { [analyze.id]: [windows.track.id, charts.track.id], [dashboards.id]: [charts.track.id] },
    });
    renderAt(`${PROJECT_PATH}/parts/${wision.id}`);

    const page = await main();
    expect(await within(page).findByRole("heading", { level: 1, name: "Wision" })).toBeTruthy();
    expect(within(page).getByText("The web app customers log in to.")).toBeTruthy();
    const features = await region("Features");
    await waitFor(() => expect(features.textContent).toContain("Being changed"));
    expect(rows(features)).toEqual(["Dashboards Widgets for latest readings Added 10 Nov", "Analyze Time-series chart per channel Being changed"]);
    expect(rows(await region("In flight"))).toEqual(["Long time windows changes Analyze 0 of 2"]);
  });
});

describe("the Docs tabs", () => {
  test("the project's Docs opens its most recently updated doc beside its docs grouped by pillar, in pillar order", async () => {
    const wision = part("Wision", "app");
    const onboarding = doc("Onboarding", "spec", { updatedAt: "2026-11-12T08:00:00Z" });
    const overview = doc("Architecture overview", "architecture", { updatedAt: "2026-11-16T08:00:00Z" });
    const routes = doc("Gateway routes", "architecture", { updatedAt: "2026-11-10T08:00:00Z" });
    const tables = doc("Readings tables", "db", { updatedAt: "2026-11-11T08:00:00Z" });
    fakeScopeApi({ parts: [wision], docs: [tables, routes, overview, onboarding, doc("Wision tokens", "ui", { part: wision.id, updatedAt: "2026-11-17T08:00:00Z" })] });
    const router = renderAt(`${PROJECT_PATH}/docs`);

    await waitFor(() => expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/docs/${overview.id}`));
    const list = await screen.findByRole("navigation", { name: "Docs" });
    expect(within(list).getAllByRole("region").map((group) => [group.getAttribute("aria-label"), within(group).getAllByRole("link").map(shownText)])).toEqual([
      ["Spec", ["Onboarding"]],
      ["Architecture", ["Architecture overview", "Gateway routes"]],
      ["Database", ["Readings tables"]],
    ]);
    expect(within(list).getByRole("link", { name: "Architecture overview" }).getAttribute("aria-current")).toBe("page");
  });

  test("a part's Docs lists the docs of the part and of its features, not other parts', and the find field narrows them by title", async () => {
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    const analyze = feature(wision.id, "Analyze");
    const compare = doc("How compare works", "architecture", { feature: analyze.id });
    const system = doc("Design system", "ui", { part: wision.id });
    fakeScopeApi({ parts: [wision, iot], features: [analyze], docs: [compare, system, doc("Readings tables", "db", { part: iot.id })] });
    renderAt(`${PROJECT_PATH}/parts/${wision.id}/docs/${compare.id}`);

    const list = await screen.findByRole("navigation", { name: "Docs" });
    await within(list).findByText("How compare works");
    expect(within(list).getAllByRole("link").map((link) => [shownText(link), link.getAttribute("href")])).toEqual([
      ["Design system", `${PROJECT_PATH}/parts/${wision.id}/docs/${system.id}`],
      ["How compare works Analyze", `${PROJECT_PATH}/parts/${wision.id}/docs/${compare.id}`],
    ]);
    fireEvent.change(within(list).getByRole("searchbox", { name: "Find a doc" }), { target: { value: "COMPARE" } });
    expect(within(list).getAllByRole("link").map(shownText)).toEqual(["How compare works Analyze"]);
  });

  test("a part's Docs with no docs opens none and says the part has none yet", async () => {
    const wision = part("Wision", "app");
    const iot = part("iot-service", "service");
    fakeScopeApi({ parts: [wision, iot], docs: [doc("Design system", "ui", { part: wision.id })] });
    const router = renderAt(`${PROJECT_PATH}/parts/${iot.id}/docs`);
    expect(await screen.findByText(/No docs on iot-service yet/)).toBeTruthy();
    expect(router.state.location.pathname).toBe(`${PROJECT_PATH}/parts/${iot.id}/docs`);
  });
});
