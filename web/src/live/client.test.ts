// US-039: the live-event client. It opens the workspace's socket
// (/ws/w/<workspace>, ws: or wss: after the page's origin), subscribes to the
// projects that views ask for, hands each project's events to its handlers in
// order and, when the socket drops, opens a new one and resubscribes every
// project with `after` set to the newest sequence number it has seen for it
// (an event's `seq`, or the `last_seq` of the `subscribed` answer), so no
// event is missed. The protocol is the API's (crates/omni-api/src/live.rs).
//
// src/live/client.ts exports
//   createLiveClient({ workspaceId, origin?, WebSocket?, reconnectDelayMs? })
// returning { subscribe(projectId, onEvent) => unsubscribe, close() }, where
// onEvent gets the event message ({ type: "event", seq, event, project_id,
// at, data }).
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

type LiveEventMessage = {
  type: "event";
  seq: number;
  event: string;
  project_id: string;
  at: string;
  data: unknown;
};

type LiveClient = {
  subscribe: (projectId: string, onEvent: (event: LiveEventMessage) => void) => () => void;
  close: () => void;
};

type ClientModule = {
  createLiveClient: (options: {
    workspaceId: string;
    origin?: string;
    WebSocket?: unknown;
    reconnectDelayMs?: number;
  }) => LiveClient;
};

/** A WebSocket the test drives: it records what the client sends. */
class FakeWebSocket extends EventTarget {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];

  readonly CONNECTING = 0;
  readonly OPEN = 1;
  readonly CLOSING = 2;
  readonly CLOSED = 3;
  readyState = FakeWebSocket.CONNECTING;
  sent: unknown[] = [];
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: Event) => void) | null = null;
  onclose: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(readonly url: string) {
    super();
    FakeWebSocket.instances.push(this);
  }

  send(data: string) {
    if (this.readyState !== FakeWebSocket.OPEN) {
      throw new Error("InvalidStateError: the socket is not open");
    }
    this.sent.push(JSON.parse(data));
  }

  close() {
    if (this.readyState === FakeWebSocket.CLOSED) return;
    this.readyState = FakeWebSocket.CLOSED;
    this.fire("close", { code: 1000, reason: "", wasClean: true });
  }

  /** The server accepted the connection. */
  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.fire("open", {});
  }

  /** The server sent a JSON text frame. */
  receive(message: unknown) {
    this.fire("message", { data: JSON.stringify(message) });
  }

  /** The connection was lost. */
  drop() {
    this.readyState = FakeWebSocket.CLOSED;
    this.fire("close", { code: 1006, reason: "", wasClean: false });
  }

  private fire(type: "open" | "message" | "close", fields: Record<string, unknown>) {
    const event = Object.assign(new Event(type), fields);
    const handler = this[`on${type}`];
    if (handler) handler.call(this, event);
    this.dispatchEvent(event);
  }
}

const WORKSPACE_ID = "0192d7a0-0000-7000-8000-00000000000a";
const PROJECT_ID = "0192d7a0-2222-7000-8000-000000000002";
const OTHER_PROJECT_ID = "0192d7a0-2222-7000-8000-000000000003";

function event(seq: number, projectId = PROJECT_ID): LiveEventMessage {
  return {
    type: "event",
    seq,
    event: "thread.created",
    project_id: projectId,
    at: "2026-10-21T10:00:00.000000Z",
    data: { thread_id: `0192d7a0-4444-7000-8000-00000000000${seq}` },
  };
}

async function loadClient(): Promise<ClientModule> {
  const specifier = "./client";
  try {
    return (await import(/* @vite-ignore */ specifier)) as ClientModule;
  } catch (error) {
    expect.fail(`src/live/client.ts cannot be imported: ${String(error)}`);
  }
}

async function connect(reconnectDelayMs = 100) {
  const { createLiveClient } = await loadClient();
  return createLiveClient({
    workspaceId: WORKSPACE_ID,
    origin: "https://omni.example.test",
    WebSocket: FakeWebSocket,
    reconnectDelayMs,
  });
}

function subscribes(socket: FakeWebSocket) {
  return socket.sent.filter(
    (message) => (message as { type?: string }).type === "subscribe",
  ) as { type: "subscribe"; project_id: string; after?: number }[];
}

/** Lets every pending reconnect timer fire. */
async function waitForReconnect() {
  await vi.advanceTimersByTimeAsync(60_000);
}

describe("live-event client", () => {
  beforeEach(() => {
    FakeWebSocket.instances = [];
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  test("it opens the workspace's socket and subscribes to a project once the socket is open", async () => {
    const client = await connect();
    client.subscribe(PROJECT_ID, () => {});
    await vi.advanceTimersByTimeAsync(0);

    expect(FakeWebSocket.instances).toHaveLength(1);
    const socket = FakeWebSocket.instances[0];
    expect(socket.url).toBe(`wss://omni.example.test/ws/w/${WORKSPACE_ID}`);
    expect(socket.sent, "nothing is sent before the socket is open").toEqual([]);

    socket.open();
    await vi.advanceTimersByTimeAsync(0);
    const sent = subscribes(socket);
    expect(sent).toHaveLength(1);
    expect(sent[0].project_id).toBe(PROJECT_ID);
    expect(sent[0].after, "a first subscription asks only for new events").toBeUndefined();
    client.close();
  });

  test("an http origin opens a ws: socket", async () => {
    const { createLiveClient } = await loadClient();
    const client = createLiveClient({
      workspaceId: WORKSPACE_ID,
      origin: "http://localhost:8080",
      WebSocket: FakeWebSocket,
    });
    client.subscribe(PROJECT_ID, () => {});
    await vi.advanceTimersByTimeAsync(0);
    expect(FakeWebSocket.instances.map((socket) => socket.url)).toEqual([
      `ws://localhost:8080/ws/w/${WORKSPACE_ID}`,
    ]);
    client.close();
  });

  test("each project's events reach that project's handlers in order", async () => {
    const client = await connect();
    const seen: LiveEventMessage[] = [];
    const seenElsewhere: LiveEventMessage[] = [];
    client.subscribe(PROJECT_ID, (e) => seen.push(e));
    client.subscribe(OTHER_PROJECT_ID, (e) => seenElsewhere.push(e));
    await vi.advanceTimersByTimeAsync(0);
    const socket = FakeWebSocket.instances[0];
    socket.open();
    await vi.advanceTimersByTimeAsync(0);
    expect(subscribes(socket).map((m) => m.project_id).sort()).toEqual(
      [PROJECT_ID, OTHER_PROJECT_ID].sort(),
    );

    socket.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 0 });
    socket.receive({ type: "subscribed", project_id: OTHER_PROJECT_ID, last_seq: 0 });
    socket.receive(event(1));
    socket.receive(event(1, OTHER_PROJECT_ID));
    socket.receive(event(2));
    await vi.advanceTimersByTimeAsync(0);

    expect(seen.map((e) => e.seq)).toEqual([1, 2]);
    expect(seen[0]).toMatchObject({
      type: "event",
      seq: 1,
      event: "thread.created",
      project_id: PROJECT_ID,
      data: event(1).data,
    });
    expect(seenElsewhere.map((e) => [e.project_id, e.seq])).toEqual([[OTHER_PROJECT_ID, 1]]);
    client.close();
  });

  test("after the socket drops it reconnects and resumes after the newest event it got", async () => {
    const client = await connect();
    const seen: number[] = [];
    client.subscribe(PROJECT_ID, (e) => seen.push(e.seq));
    await vi.advanceTimersByTimeAsync(0);
    const first = FakeWebSocket.instances[0];
    first.open();
    first.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 0 });
    first.receive(event(1));
    first.receive(event(2));
    first.receive(event(3));
    await vi.advanceTimersByTimeAsync(0);

    first.drop();
    await waitForReconnect();
    expect(FakeWebSocket.instances, "a new socket after the drop").toHaveLength(2);
    const second = FakeWebSocket.instances[1];
    expect(second.url).toBe(first.url);
    second.open();
    await vi.advanceTimersByTimeAsync(0);

    const resumed = subscribes(second);
    expect(resumed).toHaveLength(1);
    expect(resumed[0]).toMatchObject({ type: "subscribe", project_id: PROJECT_ID, after: 3 });

    second.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 5 });
    second.receive(event(4));
    second.receive(event(5));
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toEqual([1, 2, 3, 4, 5]);
    client.close();
  });

  test("a project with no events yet resumes after the last_seq of its subscription", async () => {
    const client = await connect();
    client.subscribe(PROJECT_ID, () => {});
    await vi.advanceTimersByTimeAsync(0);
    const first = FakeWebSocket.instances[0];
    first.open();
    first.receive({ type: "subscribed", project_id: PROJECT_ID, last_seq: 7 });
    await vi.advanceTimersByTimeAsync(0);

    first.drop();
    await waitForReconnect();
    const second = FakeWebSocket.instances.at(-1)!;
    expect(second).not.toBe(first);
    second.open();
    await vi.advanceTimersByTimeAsync(0);
    expect(subscribes(second)).toEqual([
      expect.objectContaining({ project_id: PROJECT_ID, after: 7 }),
    ]);
    client.close();
  });

  test("a socket that drops again keeps being replaced", async () => {
    const client = await connect();
    client.subscribe(PROJECT_ID, () => {});
    await vi.advanceTimersByTimeAsync(0);
    FakeWebSocket.instances[0].drop();
    await waitForReconnect();
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.instances[1].drop();
    await waitForReconnect();
    expect(FakeWebSocket.instances).toHaveLength(3);
    client.close();
  });

  test("a project unsubscribed from is not resubscribed after a reconnect and gets no more events", async () => {
    const client = await connect();
    const seen: number[] = [];
    const unsubscribe = client.subscribe(PROJECT_ID, (e) => seen.push(e.seq));
    client.subscribe(OTHER_PROJECT_ID, () => {});
    await vi.advanceTimersByTimeAsync(0);
    const first = FakeWebSocket.instances[0];
    first.open();
    first.receive(event(1));
    await vi.advanceTimersByTimeAsync(0);
    unsubscribe();
    first.receive(event(2));
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toEqual([1]);

    first.drop();
    await waitForReconnect();
    const second = FakeWebSocket.instances.at(-1)!;
    second.open();
    await vi.advanceTimersByTimeAsync(0);
    expect(subscribes(second).map((m) => m.project_id)).toEqual([OTHER_PROJECT_ID]);
    client.close();
  });

  test("messages that are not of the protocol are ignored", async () => {
    const client = await connect();
    const seen: number[] = [];
    client.subscribe(PROJECT_ID, (e) => seen.push(e.seq));
    await vi.advanceTimersByTimeAsync(0);
    const socket = FakeWebSocket.instances[0];
    socket.open();
    const withoutSeq: Record<string, unknown> = { ...event(1) };
    delete withoutSeq.seq;
    socket.receive(withoutSeq);
    socket.receive({ type: "error", error: "invalid_request" });
    socket.receive(event(2));
    await vi.advanceTimersByTimeAsync(0);
    expect(seen).toEqual([2]);
    client.close();
  });

  test("closing the client closes the socket and stops reconnecting", async () => {
    const client = await connect();
    client.subscribe(PROJECT_ID, () => {});
    await vi.advanceTimersByTimeAsync(0);
    const socket = FakeWebSocket.instances[0];
    socket.open();
    client.close();
    expect(socket.readyState).toBe(FakeWebSocket.CLOSED);
    await waitForReconnect();
    expect(FakeWebSocket.instances).toHaveLength(1);
  });
});
