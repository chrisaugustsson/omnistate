// @vitest-environment jsdom
// Home's API keys (/w/<ws>/api-keys): the keys agents sign in with, live ones
// first; a new key's secret shows once, in a dialog; revoking asks first.
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import * as scope from "./scope-pages-fake";

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

const KEYS = `/api/v1/w/${scope.WORKSPACE_ID}/api-keys`;
const key = (id: string, prefix: string, fields: { revoked_at?: string; last_used_at?: string } = {}) => ({
  id,
  prefix,
  created_at: scope.AT,
  scopes: [],
  last_used_at: fields.last_used_at ?? null,
  revoked_at: fields.revoked_at ?? null,
});

/** The scope fake, with the workspace's keys on top. */
function fakeKeys(initial: ReturnType<typeof key>[]) {
  scope.fakeScopeApi({});
  const base = globalThis.fetch;
  const keys = [...initial];
  const writes: { method: string; path: string }[] = [];
  const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const path = new URL(url, "http://localhost:3000").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      if (!path.startsWith(KEYS)) return base(input, init);
      if (method !== "GET") writes.push({ method, path });
      if (method === "GET") return json(200, { keys });
      if (path === KEYS) {
        const created = key("k-new", "omninew1");
        keys.push(created);
        return json(201, { key: created, secret: "omninew1.s3cret-value" });
      }
      const id = path.split("/")[6];
      const at = keys.findIndex((one) => one.id === id);
      keys[at] = { ...keys[at], revoked_at: "2026-11-18T09:00:00Z" };
      return json(200, { key: keys[at] });
    }),
  );
  return { writes };
}

beforeEach(() => {
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const rows = async () => within(await screen.findByRole("list", { name: "Keys" })).getAllByRole("listitem");

describe("API keys", () => {
  test("lists the live keys before the revoked; Create key shows the new secret once, in a dialog that Done closes", async () => {
    const api = fakeKeys([key("k-old", "omniold1", { revoked_at: "2026-11-01T00:00:00Z" }), key("k-live", "omnilive", { last_used_at: scope.AT })]);
    scope.renderAt(`/w/${scope.WORKSPACE_ID}/api-keys`);
    await waitFor(async () => expect((await rows()).map((row) => row.textContent?.slice(0, 9))).toEqual(["omnilive…", "omniold1…"]));
    expect(within((await rows())[1]).queryByRole("button", { name: "Revoke" }), "a revoked key has no Revoke").toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Create key" }));
    const dialog = await screen.findByRole("dialog", { name: "Copy your new key" });
    expect(within(dialog).getByLabelText("New key").textContent).toBe("omninew1.s3cret-value");
    expect(api.writes).toEqual([{ method: "POST", path: KEYS }]);
    fireEvent.click(within(dialog).getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByText("omninew1.s3cret-value"), "the secret is not shown again").toBeNull();
  });

  test("Revoke asks first and revokes only once confirmed", async () => {
    const api = fakeKeys([key("k-live", "omnilive")]);
    scope.renderAt(`/w/${scope.WORKSPACE_ID}/api-keys`);
    fireEvent.click(within((await rows())[0]).getByRole("button", { name: "Revoke" }));
    const dialog = screen.getByRole("dialog", { name: "Revoke omnilive…?" });
    expect(api.writes).toEqual([]);
    fireEvent.click(within(dialog).getByRole("button", { name: "Revoke key" }));
    await waitFor(() => expect(api.writes).toEqual([{ method: "POST", path: `${KEYS}/k-live/revoke` }]));
  });
});
