// @vitest-environment jsdom
// US-318: on a track whose part is a service, the UI step is hidden by
// default: the UI chip among the pillars, the UI section and, in the sidebar's
// scope tree, the UI pillar with its design docs. "Show UI" brings them all
// back together and turns into "Hide UI". The choice is kept per track for the
// browser session, so moving to another track in the app shows that track's
// own choice. On an app track the UI step shows as before, with no control.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const WISION = "0192d7a0-3333-7000-8000-000000000001";
const IOT = "0192d7a0-3333-7000-8000-000000000002";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T08:00:00Z";

let counter = 0;
/** A UUIDv7-shaped ID that no other fixture has, so no test sees another's stored choice. */
function newId() {
  counter += 1;
  return `0192d7a0-9999-7000-8000-${counter.toString(16).padStart(12, "0")}`;
}

function part(id: string, name: string, kind: string) {
  return { id, project_id: PROJECT_ID, name, kind, description: "", created_at: AT };
}

function track(name: string, partId: string) {
  return {
    track: { id: newId(), project_id: PROJECT_ID, part_id: partId, state: "planning", name, description: "", created_at: AT },
    docs: 2,
    waiting: 0,
    stories: { total: 0, open: 0, done: 0 },
    last_activity: null,
    part_name: "",
    part_deleted: false,
  };
}

type Track = ReturnType<typeof track>;

function doc(trackId: string, kind: string, pillar: string, title: string) {
  return {
    id: newId(),
    project_id: PROJECT_ID,
    track_id: trackId,
    kind,
    pillar,
    title,
    created_at: AT,
    mainline_version_id: null,
    drafts: [],
    claims: [],
  };
}

/** Each track gets a spec doc and a design in the ui pillar, titled after the track. */
function fakeApi(tracks: Track[]) {
  const docs = tracks.flatMap(({ track: t }) => [doc(t.id, "html", "spec", `${t.name}Spec`), doc(t.id, "design", "ui", `${t.name}Design`)]);
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method !== "GET") return json(404, { error: "not_found" });
      const trackAt = path.match(/\/tracks\/([^/]+)(\/features)?$/);
      if (trackAt?.[2]) return json(200, { features: [] });
      if (trackAt) {
        const one = tracks.find((summary) => summary.track.id === trackAt[1]);
        return one ? json(200, { track: one.track }) : json(404, { error: "not_found" });
      }
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", created_at: AT }] });
        case `${API}/parts`:
          return json(200, { parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")] });
        case `${API}/home`:
          return json(200, { project_id: PROJECT_ID, app: { last_activity: null }, tracks, docs, threads: [], waiting: { comments: [] } });
        case `${API}/docs`:
          return json(200, { docs });
        case `${API}/tracks`:
          return json(200, { tracks: tracks.map((summary) => summary.track) });
        case `${API}/comments`:
          return json(200, { project_id: PROJECT_ID, comments: [] });
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
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
  return router;
}

const trackPath = (summary: Track) => `${PROJECT_PATH}/tracks/${summary.track.id}`;

/** The text of `element` without its decoration (what is aria-hidden or only for screen readers). */
function shownText(element: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of Array.from(copy.querySelectorAll("[aria-hidden], .sr-only"))) hidden.remove();
  return (copy.textContent ?? "").trim();
}

/** The chips above the track page's sections: Stories, Behaviour, then the pillars and their doc counts. */
function chips() {
  const nav = within(screen.getByRole("main")).getByRole("navigation", { name: "Pillars" });
  return within(nav).getAllByRole("link").map(shownText);
}

/** The titles of the track page's sections. */
function sections() {
  return within(screen.getByRole("main"))
    .getAllByRole("region")
    .map((region) => region.getAttribute("aria-labelledby"))
    .map((id) => document.getElementById(id ?? "")?.textContent ?? "");
}

/** The rows of the sidebar's scope tree, from its head down to its Threads, as their text. */
function treeRows() {
  const nav = screen.getByRole("navigation", { name: "Project" });
  const tree = within(nav).getByRole("link", { name: /^Threads \(\d+\)$/ }).parentElement as HTMLElement;
  return Array.from(tree.querySelectorAll<HTMLElement>("a, button, div"))
    .filter((row) => row.matches("a, button") || (row.childElementCount > 0 && !row.querySelector("a, button, div")))
    .map(shownText);
}

/** The page has loaded `summary` with its part known: the chips and the tree name the track's spec doc count. */
async function loaded(summary: Track) {
  await waitFor(() => {
    expect(screen.getByRole("heading", { level: 1, name: summary.track.name })).toBeTruthy();
    expect(chips()).toContain("Spec1");
    expect(treeRows()).toContain(`${summary.track.name}Spec`);
  });
}

/** What the page and the sidebar show of the UI step of `summary`. */
function uiStep(summary: Track) {
  return {
    chip: chips().includes("UI1"),
    section: sections().includes("UI"),
    treePillar: treeRows().includes("UI1"),
    treeDesign: treeRows().includes(`${summary.track.name}Design`),
  };
}

const allShown = { chip: true, section: true, treePillar: true, treeDesign: true };
const allHidden = { chip: false, section: false, treePillar: false, treeDesign: false };

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
  sessionStorage.clear();
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  sessionStorage.clear();
  localStorage.clear();
});

describe("the UI step of a track", () => {
  test("on a service track it is hidden on the page and in the sidebar tree until Show UI, which brings it all back, and Hide UI hides it again", async () => {
    const svc = track("SvcOne", IOT);
    fakeApi([svc]);
    renderAt(trackPath(svc));
    await loaded(svc);
    const show = await screen.findByRole("button", { name: "Show UI" });
    expect(uiStep(svc), "hidden by default").toEqual(allHidden);

    fireEvent.click(show);
    await waitFor(() => expect(uiStep(svc), "Show UI brings back the chip, the section and the tree's UI").toEqual(allShown));
    expect(screen.queryByRole("button", { name: "Show UI" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Hide UI" }));
    await waitFor(() => expect(uiStep(svc), "Hide UI hides them all again").toEqual(allHidden));
    expect(screen.getByRole("button", { name: "Show UI" })).toBeTruthy();
  });

  test("the reveal stays with its service track: moving to another service track in the app shows that one hidden, and coming back shows the first still revealed", async () => {
    const one = track("SvcOne", IOT);
    const two = track("SvcTwo", IOT);
    fakeApi([one, two]);
    const router = renderAt(trackPath(one));
    await loaded(one);
    fireEvent.click(await screen.findByRole("button", { name: "Show UI" }));
    await waitFor(() => expect(uiStep(one)).toEqual(allShown));

    await router.navigate({ href: trackPath(two) });
    await loaded(two);
    expect(uiStep(two), "SvcTwo was never revealed").toEqual(allHidden);
    expect(screen.getByRole("button", { name: "Show UI" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Hide UI" })).toBeNull();

    await router.navigate({ href: trackPath(one) });
    await loaded(one);
    expect(uiStep(one), "SvcOne keeps its reveal").toEqual(allShown);
    expect(screen.getByRole("button", { name: "Hide UI" })).toBeTruthy();
  });

  test("on an app track it shows on the page and in the sidebar tree, with no Show UI or Hide UI", async () => {
    const app = track("Long windows", WISION);
    fakeApi([app]);
    renderAt(trackPath(app));
    await loaded(app);
    await waitFor(() => expect(uiStep(app)).toEqual(allShown));
    expect(screen.queryByRole("button", { name: "Show UI" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Hide UI" })).toBeNull();
  });
});
