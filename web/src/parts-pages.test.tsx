// @vitest-environment jsdom
// US-306: the project page lists its parts, each with its name and kind, and
// creates, renames and deletes one; a part's page, .../parts/<part>, lists its
// features (none until US-307) and its tracks, the active ones (planning and
// building) first, then the done and dropped ones, each group with the most
// recently updated first, each row with its state. A track created there goes
// on that part. Both pages read GET .../parts and GET .../home.
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

type Call = { method: string; path: string; body: unknown };

function part(id: string, name: string, kind: string) {
  return { id, project_id: PROJECT_ID, name, kind, description: "", created_at: AT };
}

let counter = 0;

function track(name: string, partId: string, state: string, lastActivity: string | null, createdAt = AT) {
  counter += 1;
  return {
    track: {
      id: `0192d7a0-4444-7000-8000-${counter.toString(16).padStart(12, "0")}`,
      project_id: PROJECT_ID,
      part_id: partId,
      state,
      name,
      description: "",
      created_at: createdAt,
    },
    docs: 0,
    waiting: 0,
    stories: { total: 0, open: 0, done: 0 },
    last_activity: lastActivity,
    part_name: "",
    part_deleted: false,
  };
}

type Track = ReturnType<typeof track>;

/** A fake of the API that keeps the parts, and records every write. */
function fakeApi(fields: { parts: ReturnType<typeof part>[]; tracks?: Track[] }) {
  const api = { parts: fields.parts, writes: [] as Call[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method !== "GET") {
        const body = init?.body ? JSON.parse(String(init.body)) : null;
        api.writes.push({ method, path, body });
        if (method === "POST" && path === `${API}/parts`) {
          const created = part("0192d7a0-3333-7000-8000-0000000000ff", body.name, body.kind);
          api.parts = [...api.parts, created];
          return json(201, { part: created });
        }
        if (method === "POST" && path === `${API}/tracks`)
          return json(201, { track: { ...track(body.name, body.part_id, "planning", null).track } });
        if (method === "PATCH") return json(200, { part: { ...api.parts[0], name: body.name } });
        return new Response(null, { status: 204 });
      }
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", created_at: AT }] });
        case `${API}/parts`:
          return json(200, { parts: api.parts });
        case `${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            tracks: fields.tracks ?? [],
            docs: [],
            threads: [],
            waiting: { comments: [] },
          });
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
  return router;
}

/** The text of `element` without its decoration (what is aria-hidden or only for screen readers). */
function shownText(element: HTMLElement) {
  const copy = element.cloneNode(true) as HTMLElement;
  for (const hidden of Array.from(copy.querySelectorAll("[aria-hidden], .sr-only"))) hidden.remove();
  return (copy.textContent ?? "").trim();
}

const main = () => screen.findByRole("main");
const region = async (name: string) => within(await main()).findByRole("region", { name });

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the parts on the project page", () => {
  test("each part shows its name and kind and opens its page, and New part creates one with a name and a required kind", async () => {
    const api = fakeApi({ parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")] });
    renderAt(`${PROJECT_PATH}/app`);
    const parts = await region("Parts");
    await within(parts).findByText("Wision");
    const links = within(parts).getAllByRole("link");
    expect(links.map(shownText)).toEqual(["Wisionapp", "iot-serviceservice"]);
    expect(links[0].getAttribute("href")).toBe(`${PROJECT_PATH}/parts/${WISION}`);

    fireEvent.click(within(parts).getByRole("button", { name: "New part" }));
    fireEvent.change(within(parts).getByRole("textbox", { name: "Part name" }), { target: { value: "wision-api" } });
    const create = within(parts).getByRole("button", { name: "Create part" }) as HTMLButtonElement;
    expect(create.disabled, "no kind, no create").toBe(true);
    fireEvent.change(within(parts).getByRole("combobox", { name: "Part kind" }), { target: { value: "service" } });
    fireEvent.click(create);

    await waitFor(() =>
      expect(api.writes).toEqual([{ method: "POST", path: `${API}/parts`, body: { name: "wision-api", kind: "service", description: "" } }]),
    );
    await within(parts).findByText("wision-api");
  });

  test("a part is renamed in place, and deleted only once the delete is confirmed", async () => {
    const api = fakeApi({ parts: [part(WISION, "Wision", "app")] });
    renderAt(`${PROJECT_PATH}/app`);
    const parts = await region("Parts");
    await within(parts).findByText("Wision");

    fireEvent.click(within(parts).getByRole("button", { name: "Rename Wision" }));
    fireEvent.change(within(parts).getByRole("textbox", { name: "Part name" }), { target: { value: "Wision web" } });
    fireEvent.click(within(parts).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "PATCH", path: `${API}/parts/${WISION}`, body: { name: "Wision web" } }]));

    api.writes = [];
    fireEvent.click(await within(parts).findByRole("button", { name: "Delete Wision" }));
    expect(api.writes, "asking first sends nothing").toEqual([]);
    fireEvent.click(within(parts).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "DELETE", path: `${API}/parts/${WISION}`, body: null }]));
  });
});

describe("the part page", () => {
  test("lists no features yet, and the part's tracks, active ones then done and dropped, each group most recently updated first, each with its state", async () => {
    fakeApi({
      parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")],
      tracks: [
        track("Old reports", WISION, "done", "2026-10-21T14:00:00Z"),
        track("Alarms", WISION, "planning", "2026-10-21T10:00:00Z"),
        track("Gateway", IOT, "building", "2026-10-21T13:00:00Z"),
        track("Charts", WISION, "building", "2026-10-21T12:00:00Z"),
        track("Maps", WISION, "dropped", "2026-10-21T15:00:00Z"),
        track("Login", WISION, "planning", null, "2026-10-21T11:00:00Z"),
      ],
    });
    renderAt(`${PROJECT_PATH}/parts/${WISION}`);
    expect(await within(await main()).findByRole("heading", { level: 1, name: "Wision" })).toBeTruthy();

    const features = await region("Features");
    expect(features.textContent).toContain("No features yet");

    const tracks = await region("Tracks");
    await within(tracks).findByText("Alarms");
    const rows = within(tracks).getAllByRole("link").map((row) => [row.querySelector("span span")?.textContent, within(row).getByText(/^(planning|building|done|dropped)$/).textContent]);
    expect(rows).toEqual([
      ["Charts", "building"],
      ["Login", "planning"],
      ["Alarms", "planning"],
      ["Maps", "dropped"],
      ["Old reports", "done"],
    ]);
  });

  test("New track puts the track on the page's part without asking for one", async () => {
    const api = fakeApi({ parts: [part(WISION, "Wision", "app"), part(IOT, "iot-service", "service")] });
    renderAt(`${PROJECT_PATH}/parts/${IOT}`);
    const tracks = await region("Tracks");
    fireEvent.click(within(tracks).getByRole("button", { name: "New track" }));
    expect(within(tracks).queryByRole("combobox", { name: "Part" })).toBeNull();
    fireEvent.change(within(tracks).getByRole("textbox", { name: "Track name" }), { target: { value: "Gateway" } });
    fireEvent.click(within(tracks).getByRole("button", { name: "Create track" }));
    await waitFor(() =>
      expect(api.writes).toEqual([{ method: "POST", path: `${API}/tracks`, body: { name: "Gateway", description: "", part_id: IOT } }]),
    );
  });
});
