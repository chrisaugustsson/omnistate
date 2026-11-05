// @vitest-environment jsdom
// US-14: the App page of a project renames it inline. The pencil (Rename
// project) turns the title into an input with Save and Cancel; Save sends
// PATCH /api/v1/w/<ws>/projects/<id> {name}. A 409 name_taken shows "Another
// project has this name", any other failure "The project could not be
// renamed. Try again.", both under the input. A feature's page has no pencil.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const FEATURE_ID = "0192d7a0-7777-7000-8000-000000000001";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const AT = "2026-10-23T10:00:00Z";

type Call = { method: string; path: string; body: unknown };

/** A fake of the API; `rename` answers the PATCH, and every PATCH is recorded. */
function fakeApi(rename: () => Response | Promise<Response>) {
  const patches: Call[] = [];
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const feature = { id: FEATURE_ID, project_id: PROJECT_ID, name: "Wishlist", description: "", created_at: AT };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      switch (`${method} ${path}`) {
        case "GET /api/v1/auth/me":
          return json(200, { user: USER });
        case "GET /api/v1/auth/workspaces":
          return json(200, { workspaces: [{ id: WORKSPACE_ID, name: "Acme" }] });
        case `GET /api/v1/w/${WORKSPACE_ID}/projects`:
          return json(200, { projects: [{ id: PROJECT_ID, name: "Storefront", created_at: AT }] });
        case `GET ${API}/home`:
          return json(200, {
            project_id: PROJECT_ID,
            docs: [],
            threads: [],
            waiting: { comments: [] },
            features: [{ feature, docs: 0, waiting: 0, stories: { total: 0, open: 0, done: 0 } }],
          });
        case `PATCH ${API}`:
          patches.push({ method, path, body: JSON.parse(String(init?.body)) });
          return rename();
        default:
          return json(404, { error: "not_found" });
      }
    }),
  );
  return patches;
}

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

const ok = (name: string) =>
  new Response(JSON.stringify({ project: { id: PROJECT_ID, name, created_at: AT } }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
const fail = (status: number, error: string) =>
  new Response(JSON.stringify({ error }), { status, headers: { "content-type": "application/json" } });

async function startEditing() {
  renderAt(`/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/app`);
  fireEvent.click(await screen.findByRole("button", { name: "Rename project" }));
  return (await screen.findByRole("textbox")) as HTMLInputElement;
}

describe("rename a project on its App page", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("the pencil opens a focused input with the name; Save is disabled while blank or unchanged", async () => {
    fakeApi(() => ok("x"));
    const input = await startEditing();
    expect(input.value).toBe("Storefront");
    expect(document.activeElement).toBe(input);
    const save = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "   " } });
    expect(save.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "Shop v2" } });
    expect(save.disabled).toBe(false);
  });

  test("Save sends the new name and the title shows it", async () => {
    const patches = fakeApi(() => ok("Shop v2"));
    const input = await startEditing();
    fireEvent.change(input, { target: { value: " Shop v2 " } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Shop v2"));
    expect(patches).toEqual([{ method: "PATCH", path: API, body: { name: "Shop v2" } }]);
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  test("Enter saves once, and Escape cancels without a request", async () => {
    const patches = fakeApi(() => ok("Shop v2"));
    const input = await startEditing();
    fireEvent.keyDown(input, { key: "Escape" });
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Storefront");
    expect(patches).toEqual([]);

    fireEvent.click(screen.getByRole("button", { name: "Rename project" }));
    const again = await screen.findByRole("textbox");
    fireEvent.change(again, { target: { value: "Shop v2" } });
    fireEvent.submit(again.closest("form")!);
    fireEvent.submit(again.closest("form")!);
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Shop v2"));
    expect(patches).toHaveLength(1);
  });

  test("a taken name keeps the input open with the inline error, and editing clears it", async () => {
    fakeApi(() => fail(409, "name_taken"));
    const input = await startEditing();
    fireEvent.change(input, { target: { value: "WISION" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Another project has this name");
    expect((screen.getByRole("textbox") as HTMLInputElement).value).toBe("WISION");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "WISION 2" } });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  test("any other failure shows the generic error", async () => {
    fakeApi(() => fail(500, "internal"));
    const input = await startEditing();
    fireEvent.change(input, { target: { value: "Shop v2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toBe("The project could not be renamed. Try again.");
  });

  test("a network error shows the generic error", async () => {
    fakeApi(() => Promise.reject(new TypeError("Failed to fetch")));
    const input = await startEditing();
    fireEvent.change(input, { target: { value: "Shop v2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect((await screen.findByRole("alert")).textContent).toBe("The project could not be renamed. Try again.");
  });

  test("Cancel while the save is in flight ignores its answer", async () => {
    let answer: (response: Response) => void = () => {};
    const patches = fakeApi(() => new Promise<Response>((resolve) => (answer = resolve)));
    const input = await startEditing();
    fireEvent.change(input, { target: { value: "Shop v2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(patches).toHaveLength(1));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox")).toBeNull();
    await act(async () => {
      answer(ok("Shop v2"));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Storefront");
  });

  test("a feature's page has no rename button", async () => {
    fakeApi(() => ok("x"));
    renderAt(`/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/features/${FEATURE_ID}`);
    await screen.findByRole("heading", { level: 1, name: "Wishlist" });
    expect(screen.queryByRole("button", { name: "Rename project" })).toBeNull();
  });
});
