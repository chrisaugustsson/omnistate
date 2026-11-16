// @vitest-environment jsdom
// US-324: a track's page lists its decisions, newest first, each with its question
// and decision, its why (left out when empty), who (human, or the agent's role) and
// when, with a note on one that replaces another; a feature's page lists only the
// decisions nobody replaced, each with the ones it replaced nested under it; the
// track's owner, and nobody else, adds a decision from the track page.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

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
  feature_id: string | null;
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
  links: { replaces?: number; replacedBy?: number; feature?: string | null } = {},
): Decision {
  const id = (k: number) => `0192d7a0-7777-7000-8000-${k.toString(16).padStart(12, "0")}`;
  return {
    id: id(n),
    project_id: PROJECT_ID,
    track_id: TRACK_ID,
    feature_id: links.feature === undefined ? ANALYZE : links.feature,
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
          feature_id: body.feature_id,
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
          return json(200, { decisions: api.trackDecisions });
        case `${API}/parts/${WISION}/features/${ANALYZE}`:
          return json(200, { feature: feature(ANALYZE, "Analyze") });
        case `${API}/parts/${WISION}/features/${ANALYZE}/tracks`:
          return json(200, { tracks: [trackBody] });
        case `${API}/parts/${WISION}/features/${ANALYZE}/decisions`:
          return json(200, { decisions: fields.featureDecisions ?? [] });
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
        decision(3, { question: "Which window?", decision: "Hourly", minutesAgo: 2 }, { replaces: 1 }),
        decision(2, { question: "Store raw readings?", decision: "No", why: "Too much data", author: agent("planner"), minutesAgo: 180 }),
        decision(1, { question: "Which window?", decision: "Daily", why: "Simple to start", author: agent("implementer"), minutesAgo: 2 * 24 * 60 }, { replacedBy: 3 }),
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

    expect(section.querySelectorAll("li"), "each decision shows once").toHaveLength(5);
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
    expect(newest.slice(0, 3)).toEqual(["Which window?", "Hourly", "Customers look at hours"]);
    expect(newest[3]).toMatch(/\bhuman\b/);
    expect(newest[3]).toContain("just now");
  });
});
