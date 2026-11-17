// @vitest-environment jsdom
// US-039: login and logout go through the API, and the workspace is part of
// the URL. The app is rendered with its own router (src/router.tsx) on a
// memory history, and the browser's fetch is replaced by a fake of the API's
// auth routes (crates/omni-api/src/session.rs), so the app must call the API
// from the browser:
//
// - POST /api/v1/auth/login {email, password} -> 200 {user} (sets the
//   session cookie) or 401 {"error": "invalid_credentials"}
// - POST /api/v1/auth/logout -> 204, or 401 without a session
// - GET /api/v1/auth/me -> 200 {user}, or 401 {"error": "unauthorized"}
// - GET /api/v1/auth/workspaces -> 200 {workspaces: [{id, name}]}
//
// The login page is /login: a form with "Email" and "Password" fields and a
// "Log in" button, built from shadcn/ui components. A workspace's pages are
// under /w/<workspace id>/; Log out is in the account menu at the right of the
// top bar.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const PASSWORD = "correct horse battery staple";

type Call = { method: string; path: string; body: unknown };

/** A fake of the API's auth routes with one user, and the calls it got. */
function fakeApi({ loggedIn }: { loggedIn: boolean }) {
  const api = { session: loggedIn, calls: [] as Call[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  const unauthorized = () => json(401, { error: "unauthorized" });

  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url, "http://localhost:3000").pathname;
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    let text: string | null = null;
    if (typeof init?.body === "string") text = init.body;
    else if (input instanceof Request) text = await input.clone().text();
    let body: unknown;
    try {
      body = text ? JSON.parse(text) : null;
    } catch {
      body = text;
    }
    api.calls.push({ method, path, body });

    switch (`${method} ${path}`) {
      case "POST /api/v1/auth/login": {
        const credentials = body as { email?: string; password?: string } | null;
        if (credentials?.email === USER.email && credentials?.password === PASSWORD) {
          api.session = true;
          return json(200, { user: USER });
        }
        return json(401, { error: "invalid_credentials" });
      }
      case "POST /api/v1/auth/logout":
        if (!api.session) return unauthorized();
        api.session = false;
        return new Response(null, { status: 204 });
      case "GET /api/v1/auth/me":
        return api.session ? json(200, { user: USER }) : unauthorized();
      case "GET /api/v1/auth/workspaces":
        return api.session
          ? json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] })
          : unauthorized();
      case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
        return api.session ? json(200, { projects: [] }) : unauthorized();
      default:
        return api.session ? json(404, { error: "not_found" }) : unauthorized();
    }
  });
  vi.stubGlobal("fetch", fetch);
  return api;
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({
    ...router.options,
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  render(<RouterProvider router={router} />);
  return router;
}

async function expectPath(router: ReturnType<typeof getRouter>, matches: (path: string) => boolean, what: string) {
  await waitFor(() => {
    const path = router.state.location.pathname;
    expect(matches(path), `${what}, at ${path}`).toBe(true);
  });
}

const atLogin = (path: string) => path === "/login";
const inWorkspace = (path: string) =>
  path === `/w/${WORKSPACE_ID}` || path.startsWith(`/w/${WORKSPACE_ID}/`);

async function logIn(email: string, password: string) {
  fireEvent.change(await screen.findByLabelText(/email/i), { target: { value: email } });
  fireEvent.change(screen.getByLabelText(/password/i), { target: { value: password } });
  fireEvent.click(screen.getByRole("button", { name: /log ?in|sign ?in/i }));
}

describe("login, logout and the workspace in the URL", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("a workspace page without a session sends the reviewer to the login page", async () => {
    fakeApi({ loggedIn: false });
    const router = renderAt(`/w/${WORKSPACE_ID}`);
    await expectPath(router, atLogin, "the login page");
    expect(await screen.findByLabelText(/email/i)).toBeTruthy();
    expect(screen.getByLabelText(/password/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: /log ?in|sign ?in/i })).toBeTruthy();
  });

  test("the start page without a session is the login page", async () => {
    fakeApi({ loggedIn: false });
    const router = renderAt("/");
    await expectPath(router, atLogin, "the login page");
  });

  test("logging in posts the credentials to the API and opens the workspace in the URL", async () => {
    const api = fakeApi({ loggedIn: false });
    const router = renderAt("/login");
    await logIn(USER.email, PASSWORD);

    await expectPath(router, inWorkspace, `a page of /w/${WORKSPACE_ID}`);
    const login = api.calls.filter((c) => c.method === "POST" && c.path === "/api/v1/auth/login");
    expect(login).toHaveLength(1);
    expect(login[0].body).toEqual({ email: USER.email, password: PASSWORD });
    fireEvent.click(await screen.findByRole("button", { name: /account/i }));
    expect(screen.getByRole("menuitem", { name: /log ?out|sign ?out/i })).toBeTruthy();
  });

  test("a wrong password shows the refusal and stays on the login page", async () => {
    const api = fakeApi({ loggedIn: false });
    const router = renderAt("/login");
    await logIn(USER.email, "wrong password");

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(
      api.calls.some((c) => c.method === "POST" && c.path === "/api/v1/auth/login"),
      "the login went to the API",
    ).toBe(true);
    expect(router.state.location.pathname).toBe("/login");
    expect(api.session).toBe(false);
  });

  test("the start page with a session opens the user's workspace", async () => {
    fakeApi({ loggedIn: true });
    const router = renderAt("/");
    await expectPath(router, inWorkspace, `a page of /w/${WORKSPACE_ID}`);
  });

  test("logging out ends the session through the API and returns to the login page", async () => {
    const api = fakeApi({ loggedIn: true });
    const router = renderAt(`/w/${WORKSPACE_ID}`);
    fireEvent.click(await screen.findByRole("button", { name: /account/i }));
    fireEvent.click(screen.getByRole("menuitem", { name: /log ?out|sign ?out/i }));

    await expectPath(router, atLogin, "the login page");
    expect(
      api.calls.filter((c) => c.method === "POST" && c.path === "/api/v1/auth/logout"),
    ).toHaveLength(1);
    expect(api.session).toBe(false);

    await router.navigate({ to: `/w/${WORKSPACE_ID}` as never });
    await expectPath(router, atLogin, "the login page again, the session being over");
  });
});

// US-303: a track's pages moved from .../features/... to .../tracks/... with no
// redirect, so an old URL is one the app does not know: it shows the plain
// not-found screen, not the workspace with an empty page or an error.
describe("an old /features URL after the rename to tracks", () => {
  const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
  const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
  const OLD = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/features`;

  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test.each([
    ["the list", OLD],
    ["a track's page", `${OLD}/${TRACK_ID}`],
    ["a track's board", `${OLD}/${TRACK_ID}/board`],
    ["a track's behaviour screen", `${OLD}/${TRACK_ID}/behaviour`],
  ])("the old URL of %s shows the not-found screen without the workspace around it", async (_, path) => {
    fakeApi({ loggedIn: true });
    renderAt(path);

    expect(await screen.findByText("Not Found")).toBeTruthy();
    expect(screen.queryByText("Something went wrong!")).toBeNull();
    expect(screen.queryByRole("button", { name: /account/i })).toBeNull();
  });
});
