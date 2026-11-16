// @vitest-environment jsdom
// A track's page shows who owns the track (US-313) and, for the owner and
// admins, a hand-over control listing the workspace's other members (US-315).
// Members show as the app shows them elsewhere: "You", or the user ID. Under
// the name a line shows the part the track is on, its state, its owner and the
// tracks it was split from and split off to (US-320), and the owner and admins
// get the state moves US-311 allows; a refused move shows the API's error.
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
const SOURCE_ID = "0192d7a0-4444-7000-8000-000000000002";
const SPLIT_ID = "0192d7a0-4444-7000-8000-000000000003";
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
function fakeApi(fields: { tracks: Track[]; members: Member[]; me?: string; refuse?: { status: number; error: string } }) {
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
      if (method === "POST" && path === `${API}/tracks/${TRACK_ID}/state`) {
        const body = JSON.parse(String(init?.body));
        api.posted.push({ path, body });
        if (fields.refuse) return json(fields.refuse.status, { error: fields.refuse.error, state: api.tracks[0].state });
        const moved = { ...api.tracks[0], state: body.state };
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

const details = async () => within(await screen.findByRole("main")).findByRole("group", { name: "Track details" });

/** The labels of the state moves offered, once the membership is loaded. */
async function moves() {
  await waitFor(async () => expect((await details()).textContent).toContain("Owner"));
  const group = within(await screen.findByRole("main")).queryByRole("group", { name: "Change state" });
  return group ? within(group).getAllByRole("button").map((button) => button.textContent) : [];
}

describe("the line under a track's name", () => {
  test("shows the part, linking to its page, the state, the owner, and the tracks it was split from and split off to, each linking to it", async () => {
    fakeApi({
      tracks: [
        track(TRACK_ID, "Long windows", { state: "building", split_from_track_id: SOURCE_ID }),
        track(SOURCE_ID, "Readings"),
        track(SPLIT_ID, "Aggregated readings", { split_from_track_id: TRACK_ID }),
      ],
      members: [member("m-me", USER.id)],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const line = await details();
    await waitFor(() => expect(line.textContent).toContain("Owner You"));
    expect(within(line).getByRole("link", { name: "Wision" }).getAttribute("href")).toBe(`${PROJECT_PATH}/parts/${WISION}`);
    expect(within(line).getByText("building")).toBeTruthy();
    expect(line.textContent).toContain("Split from Readings");
    expect(within(line).getByRole("link", { name: "Readings" }).getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SOURCE_ID}`);
    expect(line.textContent).toContain("Split off to Aggregated readings");
    expect(within(line).getByRole("link", { name: "Aggregated readings" }).getAttribute("href")).toBe(`${PROJECT_PATH}/tracks/${SPLIT_ID}`);
  });

  test("leaves out the split links of a track that has none", async () => {
    fakeApi({ tracks: [track(TRACK_ID, "Long windows")], members: [member("m-me", USER.id)] });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    const line = await details();
    await waitFor(() => expect(line.textContent).toContain("Owner You"));
    expect(line.textContent).not.toMatch(/split/i);
  });
});

describe("the state moves on a track page", () => {
  test("the owner of a planning track may start building or drop it, and a move goes to the state API", async () => {
    const api = fakeApi({ tracks: [track(TRACK_ID, "Long windows")], members: [member("m-me", USER.id)] });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    expect(await moves()).toEqual(["Start building", "Drop"]);
    fireEvent.click(screen.getByRole("button", { name: "Start building" }));
    await waitFor(() => expect(api.posted).toEqual([{ path: `${API}/tracks/${TRACK_ID}/state`, body: { state: "building" } }]));
  });

  test("the owner of a building track may move it back to planning, mark it done or drop it", async () => {
    fakeApi({ tracks: [track(TRACK_ID, "Long windows", { state: "building" })], members: [member("m-me", USER.id)] });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    expect(await moves()).toEqual(["Back to planning", "Mark done", "Drop"]);
  });

  test("an admin who does not own a building track may make every move but done", async () => {
    fakeApi({
      tracks: [track(TRACK_ID, "Long windows", { state: "building", owner_user_id: OTHER })],
      members: [member("m-me", USER.id, "admin"), member("m-other", OTHER)],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    expect(await moves()).toEqual(["Back to planning", "Drop"]);
  });

  test("a member who neither owns the track nor is an admin gets no moves, and neither does a done or dropped track", async () => {
    fakeApi({
      tracks: [track(TRACK_ID, "Long windows", { state: "building", owner_user_id: OTHER })],
      members: [member("m-me", USER.id), member("m-other", OTHER)],
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    expect(await moves()).toEqual([]);
    cleanup();

    for (const state of ["done", "dropped"]) {
      fakeApi({ tracks: [track(TRACK_ID, "Long windows", { state })], members: [member("m-me", USER.id)] });
      renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
      expect(await moves(), state).toEqual([]);
      cleanup();
    }
  });

  test("a move the API refuses shows its error in place", async () => {
    fakeApi({
      tracks: [track(TRACK_ID, "Long windows", { state: "building" })],
      members: [member("m-me", USER.id)],
      refuse: { status: 422, error: "invalid_transition" },
    });
    renderAt(`${PROJECT_PATH}/tracks/${TRACK_ID}`);
    await moves();
    fireEvent.click(screen.getByRole("button", { name: "Mark done" }));
    expect((await screen.findByRole("alert")).textContent).toBe("A track in building cannot move to done.");
  });
});
