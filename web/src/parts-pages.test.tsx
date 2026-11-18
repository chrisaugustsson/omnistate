// @vitest-environment jsdom
// The project's Parts (.../parts) and a part's Features (.../parts/<part>/features)
// as built in slice S2 (prototype tasks/ui-redesign/v2/parts.html,
// part-wision-features.html): a row per part with its kind, what it is, its
// features and what is in flight on it, a card per feature with what it does
// and the track changing it; creating, renaming and deleting both (US-306,
// US-310). The feature page and the features on a track page are slice S3's
// (feature-page.test.tsx, track-page.test.tsx).
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import * as scope from "./scope-pages-fake";
import { choose } from "./testing/select";

/** A WebSocket that never opens: these pages are not tested live here. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  constructor(readonly url: string) {
    super();
  }
  send() {}
  close() {
    this.readyState = FakeWebSocket.CLOSED;
  }
}

const main = () => screen.findByRole("main");

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the project's Parts", () => {
  test("a row per part shows its name, kind, what it is, how many features it has and what is in flight on it, and opens the part", async () => {
    const wision = scope.part("Wision", "app", { description: "The web app customers log in to.\nRuns on the CDN." });
    const iot = scope.part("iot-service", "service", { description: "Ingests and stores readings." });
    const api = scope.part("wision-api", "service");
    scope.fakeScopeApi({
      parts: [wision, iot, api],
      features: [scope.feature(wision.id, "Dashboards"), scope.feature(wision.id, "Analyze"), scope.feature(iot.id, "Ingest")],
      tracks: [
        scope.track("Long windows", wision.id, "planning"),
        scope.track("Exports", wision.id, "planning"),
        scope.track("Query", iot.id, "building"),
        scope.track("Charts", wision.id, "done"),
        scope.track("Moves", api.id, "dropped"),
      ],
    });
    scope.renderAt(`${scope.PROJECT_PATH}/parts`);
    const list = await within(await main()).findByRole("list", { name: "Parts" });
    await waitFor(() => expect(within(list).getAllByRole("listitem").map(scope.shownText)[0]).toContain("2 features"));
    expect(within(list).getAllByRole("listitem").map(scope.shownText)).toEqual([
      "Wision app The web app customers log in to. 2 features 2 planning",
      "iot-service service Ingests and stores readings. 1 feature 1 building",
      "wision-api service 0 features Nothing in flight",
    ]);
    expect(within(list).getByRole("link", { name: "Wision" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/parts/${wision.id}`);
  });

  test("Create part needs a name and a kind, which starts empty; a missing one is said under its field; the new part is listed", async () => {
    const api = scope.fakeScopeApi({ parts: [scope.part("Wision", "app")] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts`);
    const page = await main();
    fireEvent.click(await within(page).findByRole("button", { name: "Create part" }));
    const dialog = screen.getByRole("dialog", { name: "Create a part" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Name" }), { target: { value: "wision-api" } });
    const kind = within(dialog).getByRole("combobox", { name: "Kind" });
    expect(kind.textContent, "no kind chosen").toBe("App or service");
    fireEvent.click(within(dialog).getByRole("button", { name: "Create part" }));
    expect(within(dialog).getByRole("alert").textContent).toBe("Choose a kind");
    expect(kind.getAttribute("aria-invalid")).toBe("true");
    expect(api.writes, "no kind, no create").toEqual([]);
    choose(kind, /^Service/);
    fireEvent.change(within(dialog).getByRole("textbox", { name: /What it does/ }), { target: { value: " Users and roles " } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create part" }));

    await waitFor(() => expect(api.writes).toEqual([{ method: "POST", path: `${scope.API}/parts`, body: { name: "wision-api", kind: "service", description: "Users and roles" } }]));
    const list = within(page).getByRole("list", { name: "Parts" });
    expect(await within(list).findByRole("link", { name: "wision-api" })).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "Create a part" }), "the dialog closes").toBeNull();
  });

  test("a part is renamed in place, and deleted only once the delete is confirmed", async () => {
    const wision = scope.part("Wision", "app");
    const api = scope.fakeScopeApi({ parts: [wision] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts`);
    const list = await within(await main()).findByRole("list", { name: "Parts" });
    fireEvent.click(await within(list).findByRole("button", { name: "Rename Wision" }));
    fireEvent.change(within(list).getByRole("textbox", { name: "Part name" }), { target: { value: "Wision web" } });
    fireEvent.click(within(list).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${scope.API}/parts/${wision.id}`, body: { name: "Wision web" } }]));

    api.writes.length = 0;
    fireEvent.click(await within(list).findByRole("button", { name: "Delete Wision web" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(list).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${scope.API}/parts/${wision.id}`, body: null }]));
    await waitFor(() => expect(within(list).queryByRole("link", { name: "Wision web" })).toBeNull());
  });
});

describe("a part's Features", () => {
  test("a card per feature says what it does, the track in flight changing it or that nothing is, and how many decisions and docs it has", async () => {
    const wision = scope.part("Wision", "app");
    const dashboards = scope.feature(wision.id, "Dashboards", "Widgets for the latest readings.");
    const analyze = scope.feature(wision.id, "Analyze", "Time-series chart per channel.");
    const windows = scope.track("Long time windows", wision.id, "planning");
    const charts = scope.track("First chart", wision.id, "done");
    const raw = scope.decision("Readings are always raw", charts.track.id, { feature: analyze, part: wision }, { at: "2026-03-01T08:00:00Z" });
    const hourly = scope.decision("Aggregate per hour", windows.track.id, { feature: analyze, part: wision }, { replaces: raw.id });
    raw.replaced_by_decision_id = hourly.id;
    scope.fakeScopeApi({
      parts: [wision],
      features: [dashboards, analyze],
      tracks: [windows, charts],
      changes: { [analyze.id]: [windows.track.id, charts.track.id], [dashboards.id]: [charts.track.id] },
      decisions: [raw, hourly, scope.decision("Widgets refresh once a minute", charts.track.id, { feature: dashboards, part: wision })],
      docs: [scope.doc("How compare works", "architecture", { feature: analyze.id })],
    });
    scope.renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/features`);
    const list = await within(await main()).findByRole("list", { name: "Features" });
    await waitFor(() => expect(within(list).getAllByRole("listitem").map(scope.shownText)[1]).toContain("1 replaced"));
    expect(within(list).getAllByRole("listitem").map(scope.shownText)).toEqual([
      "Dashboards Widgets for the latest readings. Nothing in flight · added 10 Nov 1 decision 0 docs",
      "Analyze Time-series chart per channel. Being changed by Long time windows 1 decision, 1 replaced 1 doc",
    ]);
    expect(within(list).getByRole("link", { name: "Long time windows" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/tracks/${windows.track.id}`);
    expect(within(list).getByRole("link", { name: "Analyze" }).getAttribute("href")).toBe(`${scope.PROJECT_PATH}/parts/${wision.id}/features/${analyze.id}`);
  });

  test("Create feature takes a name and what it does; a feature is renamed by name only, and deleted once confirmed", async () => {
    const wision = scope.part("Wision", "app");
    const analyze = scope.feature(wision.id, "Analyze", "Look at long windows of data");
    const FEATURES = `${scope.API}/parts/${wision.id}/features`;
    const api = scope.fakeScopeApi({ parts: [wision], features: [analyze] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/features`);
    const page = await main();
    fireEvent.click(await within(page).findByRole("button", { name: "Create feature" }));
    const dialog = screen.getByRole("dialog", { name: "Create a feature" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Name" }), { target: { value: "Alarms" } });
    fireEvent.change(within(dialog).getByRole("textbox", { name: /What it does/ }), { target: { value: "Warn on thresholds" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create feature" }));
    const list = within(page).getByRole("list", { name: "Features" });
    await within(list).findByRole("link", { name: "Alarms" });
    expect(api.writes).toEqual([{ method: "POST", path: FEATURES, body: { name: "Alarms", description: "Warn on thresholds" } }]);

    api.writes.length = 0;
    fireEvent.click(within(list).getByRole("button", { name: "Rename Analyze" }));
    expect(within(list).queryByRole("textbox", { name: "Feature description" }), "no description editing").toBeNull();
    fireEvent.change(within(list).getByRole("textbox", { name: "Feature name" }), { target: { value: "Analyse" } });
    fireEvent.click(within(list).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${FEATURES}/${analyze.id}`, body: { name: "Analyse" } }]));

    api.writes.length = 0;
    fireEvent.click(await within(list).findByRole("button", { name: "Delete Analyse" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(list).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${FEATURES}/${analyze.id}`, body: null }]));
    await waitFor(() => expect(within(list).queryByRole("link", { name: "Analyse" })).toBeNull());
  });
});
