// @vitest-environment jsdom
// Home, /w/<workspace>/ (tasks/ui-redesign/build-plan.md, "Data" 6, and
// v2/home.html, v2/my-tracks.html): For you greets the signed-in member and
// lists what waits on them and their tracks, aggregated over every project of
// the workspace; My tracks lists their tracks by state; on Home the palette
// searches every project. The app is rendered with its own router on a memory
// history, against the fake API of ./testing/home-workspace.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import {
  BOB,
  CHECKOUT,
  FakeWebSocket,
  MERGED,
  MOVED,
  PICK_LISTS,
  PICK_SPEC,
  SCANNER_FLOW,
  STOREFRONT,
  WAREHOUSE,
  WS,
  fakeApi,
} from "./testing/home-workspace";

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** The rows of a list: each link's text, in order. */
const rowTexts = (list: HTMLElement) => within(list).getAllByRole("link").map((link) => link.textContent);

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

describe("For you", () => {
  test("greets the member by the name in their email, for the time of day", async () => {
    fakeApi();
    renderAt(`/w/${WS}`);
    expect((await screen.findByRole("heading", { level: 1 })).textContent).toBe("Good afternoon, Alice");
  });

  test("lists the comments waiting on the member's tracks in every project, by track, newest reply first, and nothing on other people's tracks", async () => {
    fakeApi();
    renderAt(`/w/${WS}`);
    const section = (await screen.findByRole("heading", { name: "Waiting on you" })).closest("section")!;
    await within(section).findByText("Added the scanner flow.");

    expect(within(section).getByRole("heading", { name: "Waiting on you" }).nextElementSibling?.textContent).toBe("3");
    expect(rowTexts(section).filter((text) => text !== "Open inbox" && text !== "Reply")).toEqual([
      "Pick lists",
      expect.stringContaining("Added the scanner flow."),
      "Checkout",
      expect.stringContaining("Merged the two steps into one."),
      expect.stringContaining("Moved the button above the fold."),
    ]);
    expect(section.textContent).not.toMatch(/Apple Pay and Klarna|Seven days/);
    expect(screen.getByText(/3 comments wait on you, on 2 tracks\./)).toBeTruthy();
  });

  test("a waiting comment opens where it lives, in its track's doc, and its Reply opens it in the Inbox", async () => {
    fakeApi();
    renderAt(`/w/${WS}`);
    const row = (await screen.findByText("Added the scanner flow.")).closest("a")!;
    expect(row.getAttribute("href")).toBe(`/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}/docs/${PICK_SPEC}?comment=${SCANNER_FLOW}`);
    const replies = screen.getAllByRole("link", { name: "Reply" }).map((link) => link.getAttribute("href"));
    expect(replies).toEqual([SCANNER_FLOW, MERGED, MOVED].map((id) => `/w/${WS}/inbox?comment=${id}`));
  });

  test("with nothing waiting, says the member is all caught up", async () => {
    const api = fakeApi();
    api.homes[STOREFRONT].waiting.comments = [];
    api.homes[WAREHOUSE].waiting.comments = [];
    renderAt(`/w/${WS}`);
    expect(await screen.findByText(/You’re all caught up\./)).toBeTruthy();
    expect(screen.getByText(/^Nothing waits on you\./)).toBeTruthy();
  });

  test("lists the member's tracks with their open questions, blocked mark, part and progress", async () => {
    fakeApi();
    renderAt(`/w/${WS}`);
    const list = await screen.findByLabelText("Your tracks");
    await within(list).findByText("3 questions");
    const checkout = within(list).getByText("Checkout").closest("a")!;
    expect(checkout.getAttribute("href")).toBe(`/w/${WS}/projects/${STOREFRONT}/tracks/${CHECKOUT}`);
    expect(within(checkout).getByText("3 questions")).toBeTruthy();
    expect(within(checkout).getByRole("img", { name: "US-2 blocked by Wallets US-7" })).toBeTruthy();
    expect(within(checkout).getByText("Web")).toBeTruthy();
    expect(within(checkout).getByText("1 of 4")).toBeTruthy();

    const pickLists = within(list).getByText("Pick lists").closest("a")!;
    expect(pickLists.getAttribute("href")).toBe(`/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}`);
    expect(within(pickLists).queryByText(/question/)).toBeNull();
    expect(within(pickLists).queryByRole("img", { name: /blocked/ })).toBeNull();
    expect(within(pickLists).getByText("0 of 2")).toBeTruthy();
  });
});

describe("My tracks", () => {
  /** Each band (state and count) and row (track name) of the list, in order. */
  async function listed() {
    const section = await screen.findByRole("region", { name: "My tracks by state" });
    return Array.from(section.firstElementChild!.children).map((node) =>
      node.hasAttribute("data-band") ? `band ${node.textContent}` : node.tagName === "A" ? node.querySelector("span span")?.textContent : node.textContent,
    );
  }

  test("groups the member's tracks in every project by state, each opening its track", async () => {
    fakeApi();
    renderAt(`/w/${WS}/my-tracks`);
    await screen.findByText("Pick lists");
    expect(await listed()).toEqual(["band Planning1", "Checkout", "band Building1", "Pick lists", "band Done1", "Returns", "band Dropped1", "Apple Pay"]);
    expect(screen.getByText("Pick lists").closest("a")!.getAttribute("href")).toBe(`/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}`);
  });

  test("says when a state has no tracks, and leaves Dropped out when none was dropped", async () => {
    fakeApi(BOB);
    renderAt(`/w/${WS}/my-tracks`);
    await screen.findByText("Wallets");
    expect(await listed()).toEqual(["band Planning0", "Nothing in planning.", "band Building1", "Wallets", "band Done0", "Nothing done yet."]);
  });
});

describe("the palette on Home", () => {
  test("finds tracks in every project of the workspace, not only the first", async () => {
    fakeApi();
    renderAt(`/w/${WS}`);
    await screen.findByText("Added the scanner flow.");
    fireEvent.click(screen.getByRole("button", { name: /Search/ }));
    const palette = await screen.findByRole("dialog", { name: "Search" });
    fireEvent.change(within(palette).getByRole("combobox"), { target: { value: "pick" } });
    await waitFor(() => expect(within(palette).getByRole("option", { name: "Pick lists" })).toBeTruthy());
    expect(within(palette).getByRole("option", { name: "Pick lists" }).getAttribute("href")).toBe(
      `/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}`,
    );
  });
});
