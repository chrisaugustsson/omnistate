// @vitest-environment jsdom
// The Decisions tabs of the project and of a part (slice S2; prototype
// tasks/ui-redesign/v2/decisions.html, part-wision-decisions.html): the log of
// the level and the levels under it, grouped by what each decision is about,
// with the ones a decision replaced folded under it (US-329). A track's and a
// feature's Decisions are slice S3's (object-decisions.test.tsx).
import { cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import * as scope from "./scope-pages-fake";

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

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the Decisions of the project and of a part", () => {
  const main = () => screen.findByRole("main");
  /** The groups of the log: each one's name, and its rows as the reader sees them. */
  const groups = async () =>
    within(await main())
      .getAllByRole("region")
      .map((group): [string | null, string[]] => [group.getAttribute("aria-label"), within(group).getAllByRole("listitem").filter((item) => item.parentElement?.tagName === "UL").map(scope.shownText)]);

  test("the project's log groups decisions by what they are about, the project, then each part followed by its features, newest first, and the filter keeps one kind", async () => {
    const wision = scope.part("Wision", "app");
    const iot = scope.part("iot-service", "service");
    const analyze = scope.feature(wision.id, "Analyze");
    const windows = scope.track("Long windows", wision.id, "planning");
    scope.fakeScopeApi({
      parts: [wision, iot],
      features: [analyze],
      tracks: [windows],
      decisions: [
        scope.decision("Presets only", windows.track.id, { feature: analyze, part: wision }, { question: "Custom ranges?", why: "They hid the switch.", at: "2026-11-14T08:00:00Z" }),
        scope.decision("Canvas, not SVG", windows.track.id, { part: wision }, { by: "how", question: "How to draw?", at: "2026-03-01T08:00:00Z" }),
        scope.decision("Per organisation", windows.track.id, { part: wision }, { question: "Where do settings live?", at: "2026-03-03T08:00:00Z" }),
        scope.decision("Through api-gateway", windows.track.id, { project: true }, { question: "How do apps reach services?", at: "2026-02-11T08:00:00Z" }),
        scope.decision("Only writer", windows.track.id, { part: iot }, { question: "Who writes readings?", at: "2026-11-13T08:00:00Z" }),
      ],
    });
    scope.renderAt(`${scope.PROJECT_PATH}/decisions`);
    await within(await main()).findByText("Presets only");
    expect(await groups()).toEqual([
      ["Monitoring platform", ["Through api-gateway Christopher · Answers “How do apps reach services?” · from Long windows 11 Feb"]],
      ["Wision", ["Per organisation Christopher · Answers “Where do settings live?” · from Long windows 3 Mar", "Canvas, not SVG How agent · Answers “How to draw?” · from Long windows 1 Mar"]],
      ["Analyze", ["Presets only Why: They hid the switch. Christopher · Answers “Custom ranges?” · from Long windows 14 Nov"]],
      ["iot-service", ["Only writer Christopher · Answers “Who writes readings?” · from Long windows 13 Nov"]],
    ]);

    const filter = within(await main()).getByRole("radiogroup", { name: "Decisions about" });
    expect(within(filter).getAllByRole("radio").map(scope.shownText)).toEqual(["All 5", "Project 1", "Parts 3", "Features 1"]);
    fireEvent.click(within(filter).getByRole("radio", { name: /^Parts/ }));
    expect((await groups()).map(([name]) => name)).toEqual(["Wision", "iot-service"]);
  });

  test("a decision that replaced others folds them under Replaces, the latest first; Show replaced gives each its own row too", async () => {
    const wision = scope.part("Wision", "app");
    const windows = scope.track("Long windows", wision.id, "planning");
    const raw = scope.decision("Always raw", windows.track.id, { part: wision }, { at: "2026-01-01T08:00:00Z" });
    const daily = scope.decision("Daily aggregates", windows.track.id, { part: wision }, { at: "2026-06-01T08:00:00Z", replaces: raw.id });
    const hourly = scope.decision("Hourly aggregates", windows.track.id, { part: wision }, { at: "2026-11-12T08:00:00Z", replaces: daily.id });
    raw.replaced_by_decision_id = daily.id;
    daily.replaced_by_decision_id = hourly.id;
    scope.fakeScopeApi({ parts: [wision], tracks: [windows], decisions: [raw, daily, hourly] });
    scope.renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/decisions`);
    await within(await main()).findByText("Hourly aggregates");

    const [group] = await groups();
    expect(group[1]).toHaveLength(1);
    const replaces = within(await main()).getByRole("list", { name: "Replaced decisions" });
    expect(scope.shownText(replaces.closest("details")!.querySelector("summary")!)).toBe("Replaces Daily aggregates and 1 before it");
    expect(within(replaces).getAllByRole("listitem").map((item) => item.querySelector("p")?.textContent)).toEqual(["Daily aggregates", "Always raw"]);

    fireEvent.click(within(await main()).getByRole("checkbox", { name: "Show replaced" }));
    expect((await groups())[0][1].map((row) => row.split(" Christopher")[0])).toEqual(["Hourly aggregates", "Daily aggregates replaced", "Always raw replaced"]);
  });

  test("a part's log has the part and its features, filtered one at a time, and keeps the row of one replaced by a decision about the project, marked replaced", async () => {
    const wision = scope.part("Wision", "app");
    const dashboards = scope.feature(wision.id, "Dashboards");
    const analyze = scope.feature(wision.id, "Analyze");
    const windows = scope.track("Long windows", wision.id, "planning");
    const dark = scope.decision("Dark theme", windows.track.id, { part: wision }, { at: "2026-02-01T08:00:00Z" });
    const everywhere = scope.decision("Dark everywhere", windows.track.id, { project: true }, { replaces: dark.id, at: "2026-03-01T08:00:00Z" });
    dark.replaced_by_decision_id = everywhere.id;
    scope.fakeScopeApi({
      parts: [wision],
      features: [dashboards, analyze],
      tracks: [windows],
      decisions: [dark, everywhere, scope.decision("Presets only", windows.track.id, { feature: analyze, part: wision })],
    });
    scope.renderAt(`${scope.PROJECT_PATH}/parts/${wision.id}/decisions`);
    await within(await main()).findByText("Presets only");
    const filter = within(await main()).getByRole("radiogroup", { name: "Decisions about" });
    expect(within(filter).getAllByRole("radio").map(scope.shownText)).toEqual(["All 2", "Wision 1", "Dashboards 0", "Analyze 1"]);
    expect((await groups()).map(([name, rows]) => [name, rows.map((row) => row.split(" Christopher")[0])])).toEqual([
      ["Wision", ["Dark theme replaced"]],
      ["Analyze", ["Presets only"]],
    ]);
    fireEvent.click(within(filter).getByRole("radio", { name: /^Analyze/ }));
    expect((await groups()).map(([name]) => name)).toEqual(["Analyze"]);
  });
});
