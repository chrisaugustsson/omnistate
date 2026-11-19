// @vitest-environment jsdom
// The workspace's own pages (tasks/ui-redesign/build-f.md, "Workspace"): My
// tracks lists the signed-in member's tracks in every project by state, each
// with one signal; on the workspace the palette searches every project. The
// Inbox has its own tests (./inbox.test.tsx). The app is rendered with its own
// router on a memory history, against the fake API of ./testing/home-workspace.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { BOB, FakeWebSocket, PICK_LISTS, WAREHOUSE, WS, fakeApi } from "./testing/home-workspace";

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

beforeEach(() => {
  // Wednesday 21 October 2026, three in the afternoon where the reader is.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 9, 21, 15, 0));
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("My tracks", () => {
  /** Each state band (name and count) and the tracks under it, each as its name, grey line and signal. */
  async function listed() {
    const section = await screen.findByRole("region", { name: "My tracks by state" });
    return within(section)
      .getAllByRole("list")
      .map((group) => ({
        state: `${group.getAttribute("aria-label")} ${group.previousElementSibling?.textContent?.replace(/\D+/, "")}`,
        rows: within(group)
          .getAllByRole("listitem")
          .map((item) => {
            const lines = Array.from(item.querySelectorAll("a > span:nth-child(2) > span"), (line) => line.textContent);
            const signal = item.querySelector("a > span:nth-child(3)")?.textContent ?? "";
            return lines.length > 0 ? [...lines, signal].join(" | ") : item.textContent;
          }),
      }));
  }

  test("groups the member's tracks in every project by state, each opening its track, with one signal: questions, else blocked, else progress", async () => {
    fakeApi();
    renderAt(`/w/${WS}/my-tracks`);
    await screen.findByText("3 questions");
    expect(await listed()).toEqual([
      { state: "Planning 1", rows: ["Checkout | Storefront · Web · 5 h | 3 questions"] },
      { state: "Building 1", rows: ["Pick lists | Warehouse · scanner · 6 h | 0 of 2"] },
      { state: "Done 1", rows: ["Returns | Storefront · Web · 7 h | 3 of 3"] },
      { state: "Dropped 1", rows: ["Apple Pay | Storefront · payments-api · 8 h | "] },
    ]);
    expect(screen.getByText("Pick lists").closest("a")!.getAttribute("href")).toBe(`/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}`);
  });

  test("a track with no questions waiting but a blocked story says Blocked instead of its progress", async () => {
    fakeApi();
    vi.stubGlobal("fetch", withoutQuestions(globalThis.fetch));
    renderAt(`/w/${WS}/my-tracks`);
    const checkout = (await screen.findByText("Blocked")).closest("li")!;
    expect(within(checkout).getByText("Checkout")).toBeTruthy();
    expect(within(checkout).queryByText("1 of 4")).toBeNull();
  });

  test("says when a state has no tracks, and leaves Dropped out when none was dropped", async () => {
    fakeApi(BOB);
    renderAt(`/w/${WS}/my-tracks`);
    await screen.findByText("Wallets");
    expect((await listed()).map(({ state, rows }) => [state, rows.map((text) => text?.split(" | ")[0])])).toEqual([
      ["Planning 0", ["Nothing in planning."]],
      ["Building 1", ["Wallets"]],
      ["Done 0", ["Nothing done yet."]],
    ]);
  });
});

/** The fake API, but with no behaviour questions on any track. */
function withoutQuestions(fetch: typeof globalThis.fetch): typeof globalThis.fetch {
  return async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (url.includes("/behaviour/questions")) return new Response(JSON.stringify({ questions: [] }), { status: 200, headers: { "content-type": "application/json" } });
    return fetch(input, init);
  };
}

describe("the palette on Home", () => {
  test("finds tracks in every project of the workspace, not only the first", async () => {
    fakeApi();
    renderAt(`/w/${WS}`);
    await screen.findByText("Writer agent replied on Pick list spec");
    fireEvent.click(screen.getByRole("button", { name: /Search/ }));
    const palette = await screen.findByRole("dialog", { name: "Search" });
    fireEvent.change(within(palette).getByRole("combobox"), { target: { value: "pick" } });
    await waitFor(() => expect(within(palette).getByRole("option", { name: "Pick lists" })).toBeTruthy());
    expect(within(palette).getByRole("option", { name: "Pick lists" }).getAttribute("href")).toBe(
      `/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}`,
    );
  });
});
