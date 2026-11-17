// @vitest-environment jsdom
// US-324: a track's page lists its decisions, newest first, each with its question
// and decision, its why (left out when empty), who (human, or the agent's role) and
// when, with a note on one that replaces another; a feature's page lists only the
// decisions nobody replaced, each with the ones it replaced nested under it; the
// track's owner, and nobody else, adds a decision from the track page.
// The Decisions tabs of the project and of a part (slice S2; prototype
// tasks/ui-redesign/v2/decisions.html, part-wision-decisions.html): the log of
// the level and the levels under it, grouped by what each decision is about,
// with the ones a decision replaced folded under it (US-329).
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import * as scope from "./scope-pages-fake";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const WISION = "0192d7a0-3333-7000-8000-000000000001";
const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
const ANALYZE = "0192d7a0-5555-7000-8000-000000000001";
const ALARMS = "0192d7a0-5555-7000-8000-000000000002";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const OTHER_USER_ID = "0192d7a0-1111-7000-8000-000000000002";
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T08:00:00Z";

const ago = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

type Decision = {
  id: string;
  project_id: string;
  track_id: string;
  about: string;
  feature_id: string | null;
  feature_name: string | null;
  part_id: string | null;
  part_name: string | null;
  question: string;
  decision: string;
  why: string;
  author: Record<string, unknown>;
  replaces_decision_id: string | null;
  replaced_by_decision_id: string | null;
  created_at: string;
};

function decision(
  n: number,
  fields: { question: string; decision: string; why?: string; author?: Record<string, unknown>; minutesAgo: number },
  links: { replaces?: number; replacedBy?: number; feature?: string | null; about?: "part" | "project" } = {},
): Decision {
  const id = (k: number) => `0192d7a0-7777-7000-8000-${k.toString(16).padStart(12, "0")}`;
  const featureId = links.about ? null : links.feature === undefined ? ANALYZE : links.feature;
  return {
    id: id(n),
    project_id: PROJECT_ID,
    track_id: TRACK_ID,
    about: links.about ?? (featureId ? "feature" : "track"),
    feature_id: featureId,
    feature_name: featureId === ANALYZE ? "Analyze" : null,
    part_id: links.about === "part" ? WISION : null,
    part_name: links.about === "part" ? "Wision" : null,
    question: fields.question,
    decision: fields.decision,
    why: fields.why ?? "",
    author: fields.author ?? { kind: "human", user_id: USER.id },
    replaces_decision_id: links.replaces ? id(links.replaces) : null,
    replaced_by_decision_id: links.replacedBy ? id(links.replacedBy) : null,
    created_at: ago(fields.minutesAgo),
  };
}

const agent = (role: string) => ({ kind: "agent", run: "run-7", role, model: "m", harness: "h" });

function feature(id: string, name: string) {
  return { id, project_id: PROJECT_ID, part_id: WISION, name, description: "", created_at: AT };
}

/** A fake API holding one track, owned by `owner`, seen by `me` with `role`. */
function fakeApi(fields: {
  owner: string;
  me: string;
  role?: string;
  trackDecisions?: Decision[];
  featureDecisions?: Decision[];
  partDecisions?: { decisions: Decision[]; earlier: Decision[] };
  projectDecisions?: { decisions: Decision[]; earlier: Decision[] };
  trackFeatures?: ReturnType<typeof feature>[];
}) {
  const api = { trackDecisions: fields.trackDecisions ?? [], posted: [] as unknown[] };
  const trackBody = {
    id: TRACK_ID,
    project_id: PROJECT_ID,
    part_id: WISION,
    state: "building",
    name: "Long windows",
    description: "",
    owner_user_id: fields.owner,
    created_at: AT,
  };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method === "POST" && path === `${API}/tracks/${TRACK_ID}/decisions`) {
        const body = JSON.parse(String(init?.body));
        api.posted.push(body);
        const created = {
          ...decision(99, { question: body.question, decision: body.decision, why: body.why, minutesAgo: 0 }),
          about: body.feature_id ? "feature" : "track",
          feature_id: body.feature_id,
          feature_name: body.feature_id === ANALYZE ? "Analyze" : null,
          author: { kind: "human", user_id: fields.me },
        };
        api.trackDecisions = [created, ...api.trackDecisions];
        return json(201, { decision: created });
      }
      if (method !== "GET") return new Response(null, { status: 204 });
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/whoami`:
          return json(200, { actor: { kind: "human", user_id: fields.me }, workspace_id: WORKSPACE_ID });
        case `/api/v1/w/${WORKSPACE_ID}/members`:
          return json(200, {
            members: [
              { membership_id: "m-me", user_id: fields.me, role: fields.role ?? "member" },
              ...(fields.me === fields.owner ? [] : [{ membership_id: "m-owner", user_id: fields.owner, role: "member" }]),
            ],
          });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", created_at: AT }] });
        case `${API}/parts`:
          return json(200, { parts: [{ id: WISION, project_id: PROJECT_ID, name: "Wision", kind: "app", description: "", created_at: AT }] });
        case `${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            tracks: [
              {
                track: trackBody,
                docs: 0,
                waiting: 0,
                stories: { total: 0, open: 0, done: 0 },
                last_activity: null,
                part_name: "Wision",
                part_deleted: false,
              },
            ],
            docs: [],
            threads: [],
            waiting: { comments: [] },
          });
        case `${API}/tracks`:
          return json(200, { tracks: [trackBody] });
        case `${API}/tracks/${TRACK_ID}`:
          return json(200, { track: trackBody });
        case `${API}/tracks/${TRACK_ID}/features`:
          return json(200, { features: fields.trackFeatures ?? [] });
        case `${API}/tracks/${TRACK_ID}/decisions`:
          return json(200, { decisions: api.trackDecisions, earlier: [] });
        case `${API}/parts/${WISION}/features/${ANALYZE}`:
          return json(200, { feature: feature(ANALYZE, "Analyze") });
        case `${API}/parts/${WISION}/features/${ANALYZE}/tracks`:
          return json(200, { tracks: [trackBody] });
        case `${API}/parts/${WISION}/features/${ANALYZE}/decisions`:
          return json(200, { decisions: fields.featureDecisions ?? [], earlier: [] });
        case `${API}/parts/${WISION}/features`:
          return json(200, { features: [] });
        case `${API}/parts/${WISION}/decisions`:
          return json(200, fields.partDecisions ?? { decisions: [], earlier: [] });
        case `${API}/decisions`:
          return json(200, fields.projectDecisions ?? { decisions: [], earlier: [] });
        case `${API}/docs`:
          return json(200, { docs: [] });
        case `${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
  return api;
}

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

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
}

const decisionsRegion = async () => within(await screen.findByRole("main")).findByRole("region", { name: /Decisions/ });

/** The entries directly in `list`. */
const entriesOf = (list: Element) => Array.from(list.querySelectorAll(":scope > li")) as HTMLElement[];

/** The top-level entries of the decisions section. */
const topEntries = (section: HTMLElement) => entriesOf(section.querySelector(":scope > ul")!);

/** An entry's own lines, without the decisions nested under it. */
const linesOf = (entry: HTMLElement) =>
  Array.from(entry.querySelectorAll(":scope > p")).map((line) => (line.textContent ?? "").trim());

/** The entries nested under `entry` as the ones it replaced. */
const replacedUnder = (entry: HTMLElement) => {
  const nested = entry.querySelector(":scope > ul[aria-label='Replaced decisions']");
  return nested ? entriesOf(nested) : [];
};

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("decisions on a track page", () => {
  test("are listed flat, newest first, each with question, decision, why when there is one, who and when, and a note on the one that replaces another; a non-owner admin gets no add form", async () => {
    fakeApi({
      owner: OTHER_USER_ID,
      me: USER.id,
      role: "admin",
      trackDecisions: [
        decision(3, { question: "Which window?", decision: "Hourly", minutesAgo: 2 }, { replaces: 1, feature: null }),
        decision(2, { question: "Store raw readings?", decision: "No", why: "Too much data", author: agent("planner"), minutesAgo: 180 }, { feature: null }),
        decision(1, { question: "Which window?", decision: "Daily", why: "Simple to start", author: agent("implementer"), minutesAgo: 2 * 24 * 60 }, { replacedBy: 3, feature: null }),
      ],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const section = await decisionsRegion();
    await waitFor(() => expect(topEntries(section)).toHaveLength(3));
    const [newest, middle, oldest] = topEntries(section).map(linesOf);

    expect(newest.slice(0, 2)).toEqual(["Which window?", "Hourly"]);
    expect(newest, "an empty why is left out").toHaveLength(3);
    expect(newest[2]).toMatch(/\bhuman\b/);
    expect(newest[2]).toContain("2 min ago");
    expect(newest[2]).toMatch(/replaces/);

    expect(middle.slice(0, 3)).toEqual(["Store raw readings?", "No", "Too much data"]);
    expect(middle[3]).toMatch(/\bplanner\b/);
    expect(middle[3]).not.toMatch(/human/);
    expect(middle[3]).toContain("3 h ago");
    expect(middle[3]).not.toMatch(/replaces/);

    expect(oldest.slice(0, 3)).toEqual(["Which window?", "Daily", "Simple to start"]);
    expect(oldest[3]).toMatch(/\bimplementer\b/);
    expect(oldest[3]).toContain("2 d ago");

    expect(section.querySelector("ul[aria-label='Replaced decisions']"), "the track page is a flat list").toBeNull();
    expect(within(section).queryByRole("form", { name: "Add decision" }), "only the owner gets the form").toBeNull();
    expect(within(section).queryByRole("button", { name: "Add decision" })).toBeNull();
  });
});

describe("decisions on a feature page", () => {
  test("only decisions nobody replaced are at the top, newest first, each replaced one nested once under its replacer, step by step along a chain", async () => {
    fakeApi({
      owner: USER.id,
      me: USER.id,
      featureDecisions: [
        decision(3, { question: "Which window?", decision: "Hourly", minutesAgo: 1 }, { replaces: 2 }),
        decision(10, { question: "Which colours?", decision: "Brand palette", why: "Matches the app", minutesAgo: 120 }),
        decision(2, { question: "Which window?", decision: "Daily", why: "Cheaper", minutesAgo: 24 * 60 }, { replaces: 1, replacedBy: 3 }),
        decision(11, { question: "Which unit?", decision: "Celsius", why: "Customers ask", minutesAgo: 2 * 24 * 60 }, { replacedBy: 12 }),
        decision(1, { question: "Which window?", decision: "Weekly", why: "Simple", minutesAgo: 3 * 24 * 60 }, { replacedBy: 2 }),
      ],
    });
    renderAt(`${PROJECT_PATH}/parts/${WISION}/features/${ANALYZE}`);
    const section = await decisionsRegion();
    await waitFor(() => expect(topEntries(section)).toHaveLength(3));
    const top = topEntries(section);
    expect(top.map((entry) => linesOf(entry)[1])).toEqual(["Hourly", "Brand palette", "Celsius"]);

    const underHourly = replacedUnder(top[0]);
    expect(underHourly.map((entry) => linesOf(entry)[1])).toEqual(["Daily"]);
    const underDaily = replacedUnder(underHourly[0]);
    expect(underDaily.map((entry) => linesOf(entry)[1])).toEqual(["Weekly"]);
    expect(replacedUnder(underDaily[0])).toEqual([]);
    expect(replacedUnder(top[1])).toEqual([]);
    expect(replacedUnder(top[2]), "its replacer is on another feature").toEqual([]);

    const meta = (entry: HTMLElement) => linesOf(entry).at(-1) ?? "";
    expect(meta(top[0]), "the newest says it replaces one, as on the track page").toMatch(/replaces an earlier decision/);
    expect(meta(top[0])).not.toMatch(/replaced by/);
    expect(meta(underHourly[0]), "a replaced one is marked, and says what it replaced in turn").toMatch(/replaced by a later decision/);
    expect(meta(underHourly[0])).toMatch(/replaces an earlier decision/);
    expect(meta(underDaily[0])).toMatch(/replaced by a later decision/);
    expect(meta(underDaily[0])).not.toMatch(/replaces/);
    expect(meta(top[1])).not.toMatch(/replace/);

    expect(section.querySelectorAll("li"), "each decision shows once").toHaveLength(5);
  });
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

describe("adding a decision on a track page", () => {
  test("the owner fills question, decision and why, may pick one of the track's features, and the decision shows at once", async () => {
    const api = fakeApi({
      owner: USER.id,
      me: USER.id,
      trackFeatures: [feature(ANALYZE, "Analyze"), feature(ALARMS, "Alarms")],
      trackDecisions: [decision(1, { question: "Store raw readings?", decision: "No", why: "Too much data", author: agent("planner"), minutesAgo: 60 })],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const section = await decisionsRegion();
    const form = await within(section).findByRole("form", { name: "Add decision" });
    const picker = within(form).getByLabelText("Feature") as HTMLSelectElement;
    await waitFor(() => expect(picker.options).toHaveLength(3));
    expect(picker.value, "no feature by default").toBe("");
    expect(Array.from(picker.options).map((option) => [option.value, option.textContent])).toEqual([
      ["", expect.stringMatching(/no/i)],
      [ANALYZE, "Analyze"],
      [ALARMS, "Alarms"],
    ]);

    const add = within(form).getByRole("button", { name: "Add decision" }) as HTMLButtonElement;
    fireEvent.change(within(form).getByLabelText("Question"), { target: { value: "Which window?" } });
    fireEvent.change(within(form).getByLabelText("Decision", { exact: true }), { target: { value: "Hourly" } });
    expect(add.disabled, "why is required").toBe(true);
    fireEvent.change(within(form).getByLabelText("Why"), { target: { value: "Customers look at hours" } });
    fireEvent.change(picker, { target: { value: ANALYZE } });
    expect(add.disabled).toBe(false);
    fireEvent.click(add);

    await waitFor(() => expect(topEntries(section)).toHaveLength(2));
    expect(api.posted).toEqual([{ question: "Which window?", decision: "Hourly", why: "Customers look at hours", feature_id: ANALYZE }]);
    const newest = linesOf(topEntries(section)[0]);
    expect(newest.slice(0, 4), "a label names the feature it is about").toEqual(["Feature: Analyze", "Which window?", "Hourly", "Customers look at hours"]);
    expect(newest[4]).toMatch(/\bhuman\b/);
    expect(newest[4]).toContain("just now");
  });
});
