// @vitest-environment jsdom
// A track's page shows who owns the track (US-313) and, for the owner and
// admins, a hand-over control listing the workspace's other members (US-315).
// Members show as the app shows them elsewhere: "You", or the user ID.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const WISION = "0192d7a0-3333-7000-8000-000000000001";
const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const OTHER = "0192d7a0-1111-7000-8000-000000000002";
const THIRD = "0192d7a0-1111-7000-8000-000000000003";
const PROJECT_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-21T08:00:00Z";

type Member = { membership_id: string; user_id: string; role: string };
type Track = {
  id: string;
  project_id: string;
  part_id: string;
  state: string;
  name: string;
  description: string;
  owner_user_id: string | null;
  split_from_track_id: string | null;
  created_at: string;
};

function track(id: string, name: string, fields: Partial<Track> = {}): Track {
  return {
    id,
    project_id: PROJECT_ID,
    part_id: WISION,
    state: "planning",
    name,
    description: "",
    owner_user_id: USER.id,
    split_from_track_id: null,
    created_at: AT,
    ...fields,
  };
}

const member = (membershipId: string, userId: string, role = "member"): Member => ({
  membership_id: membershipId,
  user_id: userId,
  role,
});

/** A fake API: the project's home with `tracks`, the workspace's `members`, and the logged-in user `me`. */
function fakeApi(fields: { tracks: Track[]; members: Member[]; me?: string }) {
  const api = { tracks: fields.tracks, posted: [] as { path: string; body: unknown }[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      if (method === "POST" && path === `${API}/tracks/${TRACK_ID}/owner`) {
        const body = JSON.parse(String(init?.body));
        api.posted.push({ path, body });
        const to = fields.members.find((one) => one.membership_id === body.membership_id);
        const moved = { ...api.tracks[0], owner_user_id: to?.user_id ?? null };
        api.tracks = [moved, ...api.tracks.slice(1)];
        return json(200, { track: moved });
      }
      if (method !== "GET") return new Response(null, { status: 204 });
      switch (path) {
        case "/api/v1/auth/me":
          return json(200, { user: USER });
        case "/api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `/api/v1/w/${WORKSPACE_ID}/whoami`:
          return json(200, { actor: { kind: "human", user_id: fields.me ?? USER.id }, workspace_id: WORKSPACE_ID });
        case `/api/v1/w/${WORKSPACE_ID}/members`:
          return json(200, { members: fields.members });
        case `/api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Monitoring platform", created_at: AT }] });
        case `${API}/parts`:
          return json(200, { parts: [{ id: WISION, project_id: PROJECT_ID, name: "Wision", kind: "app", description: "", created_at: AT }] });
        case `${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            app: { last_activity: null },
            tracks: api.tracks.map((one) => ({
              track: one,
              docs: 0,
              waiting: 0,
              stories: { total: 0, open: 0, done: 0 },
              last_activity: null,
              part_name: "Wision",
              part_deleted: false,
            })),
            docs: [],
            threads: [],
            waiting: { comments: [] },
          });
        case `${API}/tracks/${TRACK_ID}/features`:
          return json(200, { features: [] });
        case `${API}/tracks/${TRACK_ID}/decisions`:
          return json(200, { decisions: [], earlier: [] });
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

const ownerRegion = async () => within(await screen.findByRole("main")).findByRole("region", { name: "Owner" });

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("the owner on a track page", () => {
  test("the owner sees that they own it and a hand-over control listing the other members, and hands the track over", async () => {
    const api = fakeApi({
      tracks: [track(TRACK_ID, "Long windows")],
      members: [member("m-me", USER.id), member("m-other", OTHER), member("m-third", THIRD, "admin")],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const owner = await ownerRegion();
    await waitFor(() => expect(owner.textContent).toContain("Owned by You"));
    const picker = (await within(owner).findByLabelText("Hand over to")) as HTMLSelectElement;
    expect(Array.from(picker.options).map((option) => [option.value, option.textContent])).toEqual([
      ["", "Hand over to..."],
      ["m-other", OTHER],
      ["m-third", `${THIRD} (admin)`],
    ]);
    fireEvent.change(picker, { target: { value: "m-other" } });
    fireEvent.click(within(owner).getByRole("button", { name: "Hand over" }));
    await waitFor(() => expect(api.posted).toEqual([{ path: `${API}/tracks/${TRACK_ID}/owner`, body: { membership_id: "m-other" } }]));
  });

  test("an admin who does not own the track sees its owner and may hand it over", async () => {
    fakeApi({
      tracks: [track(TRACK_ID, "Long windows", { owner_user_id: OTHER })],
      members: [member("m-me", USER.id, "admin"), member("m-other", OTHER)],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const owner = await ownerRegion();
    await waitFor(() => expect(owner.textContent).toContain(`Owned by ${OTHER}`));
    const picker = (await within(owner).findByLabelText("Hand over to")) as HTMLSelectElement;
    expect(Array.from(picker.options).map((option) => option.value)).toEqual(["", "m-me"]);
    expect(picker.options[1].textContent).toBe("You (admin)");
  });

  test("a member who neither owns the track nor is an admin sees the owner and no hand-over control", async () => {
    fakeApi({
      tracks: [track(TRACK_ID, "Long windows", { owner_user_id: OTHER })],
      members: [member("m-me", USER.id), member("m-other", OTHER)],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const owner = await ownerRegion();
    await waitFor(() => expect(owner.textContent).toContain(`Owned by ${OTHER}`));
    expect(within(owner).queryByLabelText("Hand over to")).toBeNull();
  });

  test("the only member of the workspace still sees the owner, and that there is nobody to hand it to", async () => {
    fakeApi({ tracks: [track(TRACK_ID, "Long windows")], members: [member("m-me", USER.id)] });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const owner = await ownerRegion();
    await waitFor(() => expect(owner.textContent).toContain("Owned by You"));
    expect(owner.textContent).toContain("No other member");
  });
});
