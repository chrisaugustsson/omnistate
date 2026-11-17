// @vitest-environment jsdom
// Home's Inbox, /w/<workspace>/inbox (tasks/ui-redesign/build-plan.md, "Data":
// the Inbox is `home.waiting.comments` on tracks I own; v2/inbox.html): the
// comments waiting on the member across the workspace's projects, grouped by
// track, beside the open one, which the member answers in place with the
// comment reply API. "Everyone's" shows the comments on everyone's tracks and
// on docs of no track too. The app is rendered with its own router on a memory
// history, against the fake API of ./testing/home-workspace; live events come
// from a fake socket the test drives.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import {
  API,
  FakeWebSocket,
  MERGED,
  MOVED,
  STOREFRONT,
  WS,
  fakeApi,
} from "./testing/home-workspace";

const INBOX = `/w/${WS}/inbox`;

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

const list = () => screen.getByRole("listbox", { name: "Waiting on you" });
const openItem = () => screen.getByRole("region", { name: "Open item" });

/** The list's groups, each as its name and the first lines of its items, in order. */
function groups() {
  return within(list())
    .getAllByRole("group")
    .map((group) => ({
      name: group.getAttribute("aria-label"),
      items: within(group)
        .getAllByRole("option")
        .map((option) => option.querySelector("span span")?.textContent),
    }));
}

/** Waits until the list shows `text`. */
const listed = (text: string) => waitFor(() => expect(within(list()).getByText(text)).toBeTruthy());

beforeEach(() => {
  // 21 October 2026, three in the afternoon where the reader is.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 9, 21, 15, 0));
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  // The switch's choice outlives the page; put it back to Mine for the next test.
  const mine = screen.queryByRole("radio", { name: "Mine" });
  if (mine && mine.getAttribute("aria-checked") !== "true") fireEvent.click(mine);
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("the Inbox", () => {
  test("lists the comments waiting on the member's tracks in every project, by track, the newest reply first", async () => {
    fakeApi();
    renderAt(INBOX);
    await listed("Added the scanner flow.");
    expect(groups()).toEqual([
      { name: "Pick lists", items: ["Added the scanner flow."] },
      { name: "Checkout", items: ["Merged the two steps into one.", "Moved the button above the fold."] },
    ]);
    expect(within(screen.getByRole("region", { name: "Inbox items" })).getByText("3 waiting")).toBeTruthy();
  });

  test("Everyone's adds the comments on other people's tracks and on docs of no track, and stays chosen when the member comes back", async () => {
    fakeApi();
    const router = renderAt(INBOX);
    await listed("Added the scanner flow.");
    fireEvent.click(screen.getByRole("radio", { name: "Everyone’s" }));
    await listed("Apple Pay and Klarna.");
    expect(groups().map((group) => group.name)).toEqual(["Wallets", "Pick lists", "Checkout", "Storefront, no track"]);

    await router.navigate({ href: `/w/${WS}/my-tracks` });
    await waitFor(() => expect(screen.queryByRole("radio", { name: "Mine" })).toBeNull());
    await router.navigate({ href: INBOX });
    await listed("Seven days, as the policy says.");
    expect(screen.getByRole("radio", { name: "Everyone’s" }).getAttribute("aria-checked")).toBe("true");
  });

  test("opens the first item, with the agent's reply, the member's comment and the replies before it", async () => {
    fakeApi();
    renderAt(INBOX);
    await listed("Added the scanner flow.");
    expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Added the scanner flow.");
    expect(within(openItem()).getByText("How does the scanner start?")).toBeTruthy();

    fireEvent.click(within(list()).getByText("Merged the two steps into one."));
    await waitFor(() => expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Merged the two steps into one."));
    const conversation = within(openItem()).getByText("Your comment on Pay screens").parentElement!;
    expect(conversation.textContent).toMatch(/Drop the second step\..*I would keep it\..*Drop it anyway\./);
    const heading = within(openItem()).getByRole("heading", { level: 2 });
    expect(heading.previousElementSibling?.textContent).toBe("Designer agentreplied · 2 h");
  });

  test("a reply sent in place goes to the comment, and the comment leaves the list with the next one open", async () => {
    const api = fakeApi();
    renderAt(`${INBOX}?comment=${MERGED}`);
    await waitFor(() => expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Merged the two steps into one."));

    fireEvent.change(within(openItem()).getByRole("textbox", { name: "Reply on Pay screens" }), { target: { value: "Good, ship it." } });
    fireEvent.click(within(openItem()).getByRole("button", { name: "Send reply" }));

    await waitFor(() => expect(within(list()).queryByText("Merged the two steps into one.")).toBeNull());
    expect(api.replies).toEqual([{ path: `${API}/projects/${STOREFRONT}/comments/${MERGED}/replies`, text: "Good, ship it." }]);
    expect(groups()).toEqual([
      { name: "Pick lists", items: ["Added the scanner flow."] },
      { name: "Checkout", items: ["Moved the button above the fold."] },
    ]);
    expect(within(openItem()).getByRole("heading", { level: 2 }).textContent).toBe("Moved the button above the fold.");
  });

  test("an empty reply is not sent, and the member is asked to write one", async () => {
    const api = fakeApi();
    renderAt(INBOX);
    await listed("Added the scanner flow.");
    fireEvent.click(within(openItem()).getByRole("button", { name: "Send reply" }));
    expect(within(openItem()).getByText("Write a reply before you send it.")).toBeTruthy();
    expect(api.replies).toEqual([]);
  });

  test("a comment whose element an agent removed shows the element's snapshot", async () => {
    const api = fakeApi();
    const moved = api.homes[STOREFRONT].waiting.comments.find((comment) => comment.id === MOVED)!;
    moved.orphaned = true;
    moved.snapshot = '<li data-omni-id="l0000001">Pay with Klarna</li>';
    renderAt(`${INBOX}?comment=${MOVED}`);
    await waitFor(() => expect(within(openItem()).getByText("Element removed")).toBeTruthy());
    expect(within(openItem()).getByText("Pay with Klarna")).toBeTruthy();
  });

  test("a comment replied to elsewhere leaves the open Inbox as the reply's event comes in", async () => {
    const api = fakeApi();
    renderAt(INBOX);
    await listed("Added the scanner flow.");
    await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
    const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
    socket.open();
    await waitFor(() => expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: STOREFRONT })));
    socket.receive({ type: "subscribed", project_id: STOREFRONT, last_seq: 10 });

    const storefront = api.homes[STOREFRONT];
    storefront.waiting.comments = storefront.waiting.comments.filter((comment) => comment.id !== MOVED);
    socket.receive({
      type: "event",
      seq: 11,
      event: "comment.replied",
      project_id: STOREFRONT,
      at: new Date().toISOString(),
      data: { comment_id: MOVED, doc_id: storefront.docs[0].id },
    });
    await waitFor(() => expect(within(list()).queryByText("Moved the button above the fold.")).toBeNull());
    expect(groups().map((group) => group.items)).toEqual([["Added the scanner flow."], ["Merged the two steps into one."]]);
  });
});
