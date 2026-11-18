// @vitest-environment jsdom
// A track's Threads tab (UI redesign, slice S3): its threads in two groups,
// open and concluded, each newest post first; a thread opens in place under
// the list at …/threads/<thread>, followed live, with its posts oldest first,
// its conclusion once written and the form to post. A thread's old address
// (…/threads/<thread> of the project) opens it in its track's tab.
import { RouterProvider, createMemoryHistory } from "@tanstack/react-router";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { getRouter } from "./router";
import { API, FakeWebSocket, PROJECT_ID, PROJECT_PATH, fakeTrackApi, track } from "./testing/track-api";

const TRACK_ID = "0192d7a0-4444-7000-8000-000000000001";
const THREADS_PATH = `${PROJECT_PATH}/tracks/${TRACK_ID}/threads`;
const NOW = "2026-11-17T15:00:00.000Z";
const FONTS = "0192d7a0-8888-7000-8000-000000000001";
const WALLETS = "0192d7a0-8888-7000-8000-000000000002";
const PICKED = "0192d7a0-8888-7000-8000-000000000003";

const listed = (id: string, question: string, fields: { lastPostAt: string; posts?: number; conclusion?: string }) => ({
  id,
  project_id: PROJECT_ID,
  track_id: TRACK_ID,
  question,
  created_at: "2026-11-17T08:00:00.000Z",
  last_post_at: fields.lastPostAt,
  post_count: fields.posts ?? 1,
  conclusion: fields.conclusion ?? null,
});

const post = (id: string, threadId: string, summary: string, author: Record<string, unknown>, createdAt: string) => ({
  id,
  thread_id: threadId,
  author,
  summary,
  body: summary,
  body_html: `<p>${summary}</p>`,
  links: [],
  attachments: [],
  created_at: createdAt,
});

const AGENT = { kind: "agent", run: "r1", role: "critic", model: "m", harness: "h" };
const HUMAN = { kind: "human", user_id: "u" };

/** The Wallets thread in full: an agent's post, then one of yours. */
function walletsBody(extra: ReturnType<typeof post>[] = [], conclusion: string | null = null) {
  return {
    id: WALLETS,
    project_id: PROJECT_ID,
    track_id: TRACK_ID,
    question: "Which wallets do we take?",
    created_at: "2026-11-17T08:00:00.000Z",
    conclusion,
    posts: [
      post("p1", WALLETS, "Apple Pay covers most phones", AGENT, "2026-11-17T12:00:00.000Z"),
      post("p2", WALLETS, "Add Swish too", HUMAN, "2026-11-17T14:00:00.000Z"),
      ...extra,
    ],
  };
}

function threadsApi() {
  return fakeTrackApi({
    tracks: [track(TRACK_ID, "Checkout")],
    threads: [
      listed(FONTS, "Which fonts?", { lastPostAt: "2026-11-17T11:00:00.000Z" }),
      listed(WALLETS, "Which wallets do we take?", { lastPostAt: "2026-11-17T14:00:00.000Z", posts: 2 }),
      listed(PICKED, "Which colours?", { lastPostAt: "2026-11-17T13:00:00.000Z", conclusion: "Brand palette." }),
    ],
    threadBodies: { [WALLETS]: walletsBody() },
  });
}

function renderAt(path: string) {
  const router = getRouter();
  router.update({ ...router.options, history: createMemoryHistory({ initialEntries: [path] }) });
  render(<RouterProvider router={router} />);
  return router;
}

const rowQuestions = (region: HTMLElement) =>
  within(region)
    .queryAllByRole("link")
    .map((row) => Array.from(row.querySelectorAll("span")).find((span) => span.textContent)?.textContent);

async function openThread() {
  return screen.findByRole("article", { name: "Which wallets do we take?" });
}

const postSummaries = (article: HTMLElement) =>
  Array.from(article.querySelectorAll<HTMLElement>('li[id^="post-"]')).map((item) => within(item).getByRole("heading", { level: 3 }).textContent);

async function liveSocket() {
  await waitFor(() => expect(FakeWebSocket.instances.length).toBeGreaterThan(0));
  const socket = FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  socket.open();
  await waitFor(() => expect(socket.sent).toContainEqual(expect.objectContaining({ type: "subscribe", project_id: PROJECT_ID })));
  socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 20 });
  return socket;
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(NOW));
  vi.stubGlobal("scrollTo", () => {});
  FakeWebSocket.instances = [];
  vi.stubGlobal("WebSocket", FakeWebSocket);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("a track's Threads tab", () => {
  test("lists the open threads apart from the concluded ones, each newest post first, and counts both", async () => {
    threadsApi();
    renderAt(THREADS_PATH);
    const open = await screen.findByRole("region", { name: "Open threads" });
    await waitFor(() => expect(rowQuestions(open)).toEqual(["Which wallets do we take?", "Which fonts?"]));
    expect(rowQuestions(screen.getByRole("region", { name: "Concluded threads" }))).toEqual(["Which colours?"]);
    expect(screen.getByText("2 open").closest("p")?.textContent).toBe("2 open · 1 concluded");
    expect(screen.queryByRole("article")).toBeNull();
  });

  test("a thread opens in place under the list at its own address, its row current, its posts oldest first", async () => {
    threadsApi();
    const router = renderAt(THREADS_PATH);
    const open = await screen.findByRole("region", { name: "Open threads" });
    fireEvent.click(await within(open).findByRole("link", { name: /Which wallets do we take\?/ }));

    const article = await openThread();
    expect(router.state.location.pathname).toBe(`${THREADS_PATH}/${WALLETS}`);
    const listed = within(screen.getByRole("region", { name: "Open threads" }));
    expect(listed.getByRole("link", { name: /Which wallets do we take\?/ }).getAttribute("aria-current")).toBe("page");
    expect(listed.getByRole("link", { name: /Which fonts\?/ }).getAttribute("aria-current")).toBeNull();
    await waitFor(() => expect(postSummaries(article)).toEqual(["Apple Pay covers most phones", "Add Swish too"]));
    expect(within(article).queryByRole("region", { name: "Conclusion" })).toBeNull();
  });

  test("a concluded thread shows its conclusion", async () => {
    const api = threadsApi();
    api.fixture.threadBodies = { [WALLETS]: walletsBody([], "We take Apple Pay and Swish.") };
    renderAt(`${THREADS_PATH}/${WALLETS}`);
    const article = await openThread();
    expect((await within(article).findByRole("region", { name: "Conclusion" })).textContent).toContain("We take Apple Pay and Swish.");
  });

  test("a thread from an agent run names the run in its meta line", async () => {
    const api = threadsApi();
    api.fixture.threadBodies = { [WALLETS]: { ...walletsBody(), run_tag: "seed" } };
    renderAt(`${THREADS_PATH}/${WALLETS}`);
    const article = await openThread();
    const meta = (await within(article).findByText(/^Started/)).closest("p");
    expect(meta?.textContent).toContain("Started 17 Nov· run seed");
  });

  test("your post is sent as written and the fields clear", async () => {
    const api = threadsApi();
    renderAt(`${THREADS_PATH}/${WALLETS}`);
    const article = await openThread();
    fireEvent.change(within(article).getByRole("textbox", { name: "Summary" }), { target: { value: " Add Klarna " } });
    fireEvent.change(within(article).getByRole("textbox", { name: "Body" }), { target: { value: "- Germany\n" } });
    fireEvent.click(within(article).getByRole("button", { name: "Post" }));

    await waitFor(() => expect((within(article).getByRole("textbox", { name: "Summary" }) as HTMLInputElement).value).toBe(""));
    expect(api.calls.filter((call) => call.method === "POST").map(({ path, body }) => ({ path, body }))).toEqual([
      { path: `${API}/threads/${WALLETS}/posts`, body: { summary: " Add Klarna ", body: "- Germany\n" } },
    ]);
  });

  test("a post that arrives live is added at the bottom of the open thread", async () => {
    const api = threadsApi();
    renderAt(`${THREADS_PATH}/${WALLETS}`);
    const article = await openThread();
    await waitFor(() => expect(postSummaries(article)).toHaveLength(2));
    const socket = await liveSocket();

    api.fixture.threadBodies = { [WALLETS]: walletsBody([post("p3", WALLETS, "Klarna is wanted in Germany", AGENT, "2026-11-17T14:59:00.000Z")]) };
    socket.receive({ type: "event", seq: 21, event: "post.created", project_id: PROJECT_ID, at: NOW, data: { post_id: "p3", thread_id: WALLETS } });

    await waitFor(() => expect(postSummaries(article)).toEqual(["Apple Pay covers most phones", "Add Swish too", "Klarna is wanted in Germany"]));
  });

  test("#post-<id> scrolls to that post and highlights it", async () => {
    const scrolled: string[] = [];
    Element.prototype.scrollIntoView = vi.fn(function (this: Element) {
      scrolled.push(this.id);
    });
    threadsApi();
    renderAt(`${THREADS_PATH}/${WALLETS}#post-p1`);
    const article = await openThread();
    await waitFor(() => expect(scrolled).toContain("post-p1"));
    expect(scrolled).not.toContain("post-p2");
    expect(article.querySelector("#post-p1 [data-highlighted]")).toBeTruthy();
    expect(article.querySelector("#post-p2 [data-highlighted]")).toBeNull();
  });
});

describe("a thread's old address", () => {
  test("opens the thread in its track's Threads tab", async () => {
    threadsApi();
    const router = renderAt(`${PROJECT_PATH}/threads/${WALLETS}`);
    await openThread();
    expect(router.state.location.pathname).toBe(`${THREADS_PATH}/${WALLETS}`);
  });
});
