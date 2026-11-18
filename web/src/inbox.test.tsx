// @vitest-environment jsdom
// The workspace's Inbox, /w/<workspace> (tasks/ui-redesign/build-f.md,
// "Workspace = Inbox"; nav5/option-f.html, screen 1): what waits on the member
// across the workspace's projects, newest first, as a list with the open item
// beside it. A comment waits when an agent replied last on a track the member
// owns; the member answers it or resolves it in place. A track of theirs with
// behaviour questions an agent asked is a row too. The open row is in the URL.
// On the phone the list is the page. The app is rendered with its own router
// on a memory history, against the fake API of ./testing/home-workspace.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { API, CHECKOUT, FakeWebSocket, MERGED, MOVED, PICK_LISTS, SCANNER_FLOW, STOREFRONT, WAREHOUSE, WS, fakeApi } from "./testing/home-workspace";

const INBOX = `/w/${WS}`;

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

/** Makes the window phone-wide (under 768px) or not. */
function windowWidth(phone: boolean) {
  vi.stubGlobal("matchMedia", (media: string) => ({ media, matches: phone, addEventListener() {}, removeEventListener() {} }));
}

const list = () => screen.getByRole("list", { name: "Waiting on you" });
const openItem = () => screen.getByRole("region", { name: "Open item" });

/** The list's rows, each as its two lines. */
function rows() {
  return within(list())
    .getAllByRole("link")
    .map((row) => Array.from(row.querySelectorAll("span span"), (line) => line.textContent));
}

/** The row whose first line is `title`, the `nth` of them. */
const row = (title: string, nth = 0) => within(list()).getAllByText(title)[nth].closest("a")!;

/** The open item's thread: each message's text, in order. */
const thread = () =>
  within(within(openItem()).getByRole("list", { name: "Thread" }))
    .getAllByRole("listitem")
    .map((message) => message.querySelector("p")?.textContent);

beforeEach(() => {
  // 21 October 2026, three in the afternoon where the reader is.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 9, 21, 15, 0));
  vi.stubGlobal("scrollTo", () => {});
  vi.stubGlobal("WebSocket", FakeWebSocket);
  windowWidth(false);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the Inbox", () => {
  test("lists what waits on the member across projects, newest first: who replied on which doc over the track and when, and their track's behaviour questions", async () => {
    fakeApi();
    renderAt(INBOX);
    await screen.findByText("3 behaviour questions");
    expect(rows()).toEqual([
      ["Writer agent replied on Pick list spec", "Pick lists · 1 h"],
      ["Designer agent replied on Pay screens", "Checkout · 2 h"],
      ["Designer agent replied on Pay screens", "Checkout · 4 h"],
      ["3 behaviour questions", "Checkout · 1 Oct"],
    ]);
  });

  test("opens the newest item, and a row opens its own: the doc's quoted text and the whole thread, kept in the URL so Back returns", async () => {
    fakeApi();
    const router = renderAt(INBOX);
    await screen.findByText("3 behaviour questions");
    expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Pick list spec");
    expect(thread()).toEqual(["How does the scanner start?", "Added the scanner flow."]);

    fireEvent.click(row("Designer agent replied on Pay screens"));
    await waitFor(() => expect(router.state.location.search).toEqual({ comment: MERGED }));
    expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Pay screens");
    expect(thread()).toEqual(["Drop the second step.", "I would keep it.", "Drop it anyway.", "Merged the two steps into one."]);
    expect(await within(openItem()).findByText("“Pay with card, in one step.”")).toBeTruthy();
    expect(row("Designer agent replied on Pay screens").getAttribute("aria-current")).toBe("page");

    router.history.back();
    await waitFor(() => expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Pick list spec"));
  });

  test("a reply goes to the comment, which leaves the list, and the next item opens", async () => {
    const api = fakeApi();
    renderAt(`${INBOX}?comment=${MERGED}`);
    await waitFor(() => expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Pay screens"));

    fireEvent.change(within(openItem()).getByRole("textbox", { name: "Reply on Pay screens" }), { target: { value: "Good, ship it." } });
    fireEvent.click(within(openItem()).getByRole("button", { name: "Reply" }));

    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(api.replies).toEqual([{ path: `${API}/projects/${STOREFRONT}/comments/${MERGED}/replies`, text: "Good, ship it." }]);
    expect(rows().map(([title, sub]) => `${title} / ${sub}`)).not.toContain("Designer agent replied on Pay screens / Checkout · 2 h");
    expect(thread().at(-1)).toBe("Moved the button above the fold.");
  });

  test("an empty reply is not sent, and asks for words", async () => {
    const api = fakeApi();
    renderAt(`${INBOX}?comment=${MOVED}`);
    await waitFor(() => expect(thread().at(-1)).toBe("Moved the button above the fold."));
    fireEvent.click(within(openItem()).getByRole("button", { name: "Reply" }));
    expect(within(openItem()).getByText("Write a reply before you send it.")).toBeTruthy();
    expect(api.replies).toEqual([]);
  });

  test("Resolve with nothing written closes the comment with “Resolved.”, and it leaves the list", async () => {
    const api = fakeApi();
    renderAt(`${INBOX}?comment=${MOVED}`);
    await waitFor(() => expect(thread().at(-1)).toBe("Moved the button above the fold."));
    fireEvent.click(within(openItem()).getByRole("button", { name: "Resolve" }));

    await waitFor(() => expect(rows()).toHaveLength(3));
    expect(api.replies).toEqual([{ path: `${API}/projects/${STOREFRONT}/comments/${MOVED}/replies`, text: "Resolved." }]);
  });

  test("Open in track opens the track's section that shows the doc, with the comment open", async () => {
    fakeApi();
    renderAt(`${INBOX}?comment=${MERGED}`);
    const open = await within(await screen.findByRole("region", { name: "Open item" })).findByRole("link", { name: "Open in track" });
    expect(open.getAttribute("href")).toBe(`/w/${WS}/projects/${STOREFRONT}/tracks/${CHECKOUT}/design?comment=${MERGED}`);

    fireEvent.click(row("Writer agent replied on Pick list spec"));
    await waitFor(() =>
      expect(within(openItem()).getByRole("link", { name: "Open in track" }).getAttribute("href")).toBe(
        `/w/${WS}/projects/${WAREHOUSE}/tracks/${PICK_LISTS}?comment=${SCANNER_FLOW}`,
      ),
    );
  });

  test("the behaviour questions row opens the questions an agent asked, and leads to the track's Behaviour", async () => {
    fakeApi();
    const router = renderAt(INBOX);
    await screen.findByText("3 behaviour questions");
    fireEvent.click(row("3 behaviour questions"));
    await waitFor(() => expect(router.state.location.search).toEqual({ questions: CHECKOUT }));
    expect(within(openItem()).getByText("Question 1?")).toBeTruthy();
    expect(within(openItem()).queryByText("Question 3?")).toBeNull();
    expect(within(openItem()).getByRole("link", { name: "Answer on Behaviour" }).getAttribute("href")).toBe(
      `/w/${WS}/projects/${STOREFRONT}/tracks/${CHECKOUT}/behaviour`,
    );
  });

  test("with nothing waiting, says so quietly and leads to My tracks", async () => {
    const api = fakeApi();
    for (const home of Object.values(api.homes)) home.waiting.comments = [];
    vi.stubGlobal("fetch", wrapQuestionsEmpty(globalThis.fetch));
    renderAt(INBOX);
    const line = await screen.findByText(/^Nothing waits on you\./);
    expect(within(line).getByRole("link", { name: "My tracks" }).getAttribute("href")).toBe(`/w/${WS}/my-tracks`);
    expect(screen.queryByRole("region", { name: "Open item" })).toBeNull();
  });
});

describe("the Inbox on the phone", () => {
  test("the list is the page, a row opens its item full screen, and ‹ Inbox goes back to the list", async () => {
    windowWidth(true);
    fakeApi();
    const router = renderAt(INBOX);
    await screen.findByText("3 behaviour questions");
    expect(screen.queryByRole("region", { name: "Open item" })).toBeNull();

    fireEvent.click(row("Designer agent replied on Pay screens"));
    await waitFor(() => expect(thread().at(-1)).toBe("Merged the two steps into one."));
    expect(screen.queryByRole("list", { name: "Waiting on you" })).toBeNull();

    fireEvent.click(within(openItem()).getByRole("link", { name: "Inbox" }));
    await waitFor(() => expect(router.state.location.search).toEqual({}));
    expect(rows()).toHaveLength(4);
    expect(screen.queryByRole("region", { name: "Open item" })).toBeNull();
  });
});

/** The fake API, but with no behaviour questions on any track. */
function wrapQuestionsEmpty(fetch: typeof globalThis.fetch): typeof globalThis.fetch {
  return async (input, init) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    if (url.includes("/behaviour/questions")) return new Response(JSON.stringify({ questions: [] }), { status: 200, headers: { "content-type": "application/json" } });
    return fetch(input, init);
  };
}
