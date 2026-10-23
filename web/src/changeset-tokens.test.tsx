// @vitest-environment jsdom
// The change set review shows what a change set changes in the design
// system's tokens. A design-system doc's node changes say nothing about its
// tokens (a new accent colour is "0 changed"), so its entry in the review's
// "Docs" section loads the token tree of the base version and of the version
// under review (GET /projects/<project>/tokens?version=<id>, the tree with a
// `$value` per token and its value per mode in
// `$extensions.omnistate.modes`) and lists the difference:
//
// - a region named "Tokens" inside the doc's entry, with one row (a list item
//   or a table row) per added, changed or removed token, holding the token's
//   path ("color.accent"), the word "added", "changed" or "removed", and its
//   values: the old and new value of each mode for a changed token, the new
//   ones for an added token and the old ones for a removed token. Unchanged
//   tokens are not listed.
// - the counts of the token changes in that region: "<n> added",
//   "<n> changed" and "<n> removed".
// - a design system without a base version (new in the change set) lists
//   every token as added.
//
// The app is rendered with its own router on a memory history, the browser's
// fetch is replaced by a fake of the API, and the browser's WebSocket by a
// fake socket.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000001";
const CS_ID = "0192d7a0-8888-7000-8000-000000000011";
const SYSTEM_DOC = "0192d7a0-8888-7000-8000-000000000003";
const SYSTEM_BASE = "0192d7a0-8888-7000-8000-000000000025";
const SYSTEM_NEW = "0192d7a0-8888-7000-8000-000000000026";
const USER = { id: "0192d7a0-1111-7000-8000-000000000001", email: "alice@example.com" };
const AT = "2026-10-21T10:00:00Z";
const API = `/api/v1/w/${WORKSPACE_ID}/projects/${PROJECT_ID}`;
const REVIEW_PATH = `/w/${WORKSPACE_ID}/projects/${PROJECT_ID}/changesets/${CS_ID}`;
const CS_TITLE = "Warmer booking button";
const MODES = ["Ljust", "Mörkt"];

/** A token with a value per mode; `$value` is the first mode's. */
function token(light: string, dark: string) {
  return { $value: light, $extensions: { omnistate: { modes: { Ljust: light, Mörkt: dark } } } };
}

const BASE_TOKENS = {
  color: {
    accent: token("#00aa8a", "#01b597"),
    surface: token("#ffffff", "#131316"),
  },
  space: {
    huge: { $value: "64px" },
    small: { $value: "8px" },
  },
};

const NEW_TOKENS = {
  color: {
    accent: token("#d9480f", "#e8590c"),
    surface: token("#ffffff", "#131316"),
    warning: token("#f59f00", "#fab005"),
  },
  space: {
    small: { $value: "8px" },
  },
};

function reviewedSystem(baseVersion: string | null) {
  return {
    doc_id: SYSTEM_DOC,
    kind: "design-system",
    module: "ui",
    title: "Design system",
    base_version_id: baseVersion,
    version_id: SYSTEM_NEW,
    removed: false,
    changes: { added: [], removed: [], changed: [], moved: [] },
  };
}

function fakeApi({ baseVersion }: { baseVersion: string | null }) {
  const api = { tokenReads: [] as string[] };
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const changeset = {
    id: CS_ID,
    project_id: PROJECT_ID,
    feature_id: null,
    title: CS_TITLE,
    description: "A warmer accent so the booking button stands out.",
    status: "in_review",
    run_tag: "run-1",
    created_at: AT,
    history: [{ from_status: null, to_status: "in_review", actor: "agent", reason: null, at: AT }],
    approved_versions: [],
    decisions: [],
  };
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url, "http://localhost:3000");
    const method = (init?.method ?? (input instanceof Request ? input.method : "GET")).toUpperCase();
    const path = url.pathname;
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
          changesets: [changeset],
          threads: [],
          waiting: { decisions: [], changesets: [changeset], comments: [], questions: [] },
          features: [],
        });
      case `GET ${API}/changesets/${CS_ID}/review`:
        return json(200, { changeset, decisions: [], docs: [reviewedSystem(baseVersion)] });
      case `GET ${API}/changesets/${CS_ID}/conflicts`:
        return json(200, { conflicts: [] });
      case `GET ${API}/tokens`: {
        const version = url.searchParams.get("version");
        api.tokenReads.push(version ?? "mainline");
        const tokens = version === SYSTEM_BASE ? BASE_TOKENS : version === SYSTEM_NEW ? NEW_TOKENS : null;
        if (!tokens) return json(404, { error: "not_found" });
        return json(200, { project_id: PROJECT_ID, doc_id: SYSTEM_DOC, version_id: version, modes: MODES, tokens });
      }
    }
    return json(404, { error: "not_found" });
  });
  vi.stubGlobal("fetch", fetch);
  return api;
}

/** A WebSocket that stays connecting: the review needs no live events. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  constructor(readonly url: string) {
    super();
  }
  send() {
    throw new Error("InvalidStateError: the socket is not open");
  }
  close() {
    this.readyState = FakeWebSocket.CLOSED;
  }
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
}

/** The "Tokens" region of the design system's entry in the review. */
async function tokensRegion() {
  await screen.findByRole("heading", { name: new RegExp(CS_TITLE, "i") }, { timeout: 3000 });
  const docs = await screen.findByRole("region", { name: /^(docs|changes|changed docs)$/i }, { timeout: 3000 });
  const entry = await waitFor(() => {
    const found = within(docs).getAllByText("Design system")[0].closest("li, article, [role=listitem]");
    expect(found, "the design system's entry is a list item or an article").toBeTruthy();
    return found as HTMLElement;
  });
  return within(entry).findByRole("region", { name: /^tokens\b/i }, { timeout: 3000 });
}

/** The row of `region` that shows the token `path`. */
function rowOf(region: HTMLElement, path: string) {
  const named = within(region).getByText(path);
  const row = named.closest("li, tr, [role=row], [role=listitem]");
  expect(row, `the token ${path} has a row`).toBeTruthy();
  return row as HTMLElement;
}

describe("design token changes in the change set review", () => {
  beforeEach(() => {
    vi.stubGlobal("scrollTo", () => {});
    vi.stubGlobal("WebSocket", FakeWebSocket);
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  test("a changed token shows its old and new value in each mode", async () => {
    const api = fakeApi({ baseVersion: SYSTEM_BASE });
    renderAt(REVIEW_PATH);
    const region = await tokensRegion();
    const accent = await waitFor(() => rowOf(region, "color.accent"));
    const text = accent.textContent ?? "";
    expect(text).toMatch(/\bchanged\b/i);
    for (const value of ["#00aa8a", "#d9480f", "#01b597", "#e8590c", "Ljust", "Mörkt"]) {
      expect(text, `the accent's row shows ${value}`).toContain(value);
    }
    expect(text.indexOf("#00aa8a"), "the old light value comes before the new one").toBeLessThan(text.indexOf("#d9480f"));
    expect(api.tokenReads).toEqual(expect.arrayContaining([SYSTEM_BASE, SYSTEM_NEW]));
  });

  test("added and removed tokens are listed, unchanged ones are not, and the changes are counted", async () => {
    fakeApi({ baseVersion: SYSTEM_BASE });
    renderAt(REVIEW_PATH);
    const region = await tokensRegion();
    const warning = await waitFor(() => rowOf(region, "color.warning"));
    expect(warning.textContent).toMatch(/\badded\b/i);
    expect(warning.textContent).toContain("#f59f00");
    const huge = rowOf(region, "space.huge");
    expect(huge.textContent).toMatch(/\bremoved\b/i);
    expect(huge.textContent).toContain("64px");

    expect(within(region).queryByText("color.surface"), "an unchanged colour is not listed").toBeNull();
    expect(within(region).queryByText("space.small"), "an unchanged size is not listed").toBeNull();

    const counts = region.textContent ?? "";
    expect(counts).toMatch(/1\s*added/i);
    expect(counts).toMatch(/1\s*changed/i);
    expect(counts).toMatch(/1\s*removed/i);
  });

  test("a design system new in the change set lists every token as added", async () => {
    const api = fakeApi({ baseVersion: null });
    renderAt(REVIEW_PATH);
    const region = await tokensRegion();
    for (const path of ["color.accent", "color.surface", "color.warning", "space.small"]) {
      const row = await waitFor(() => rowOf(region, path));
      expect(row.textContent, `${path} is added`).toMatch(/\badded\b/i);
    }
    expect(region.textContent ?? "").toMatch(/4\s*added/i);
    expect(api.tokenReads, "no base version is read").toEqual([SYSTEM_NEW]);
  });
});
