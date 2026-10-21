import { LiveServerMessageSchema, type LiveServerMessage } from "@/api/generated";

/** An event message of the live socket, as `src/api/generated.ts` types it. */
export type LiveEvent = Extract<LiveServerMessage, { type: "event" }>;

/** The part of the browser's WebSocket that the client uses. */
export type SocketLike = {
  readyState: number;
  send(data: string): void;
  close(): void;
  addEventListener(type: "open" | "close" | "error", listener: () => void): void;
  addEventListener(type: "message", listener: (event: { data: unknown }) => void): void;
};

export type LiveClientOptions = {
  workspaceId: string;
  /** The page's origin by default; `http:` gives a `ws:` socket and `https:` a `wss:` one. */
  origin?: string;
  WebSocket?: new (url: string) => SocketLike;
  /** The first wait before a new socket is opened; it doubles while attempts fail. */
  reconnectDelayMs?: number;
};

export type LiveClient = {
  /** Gets every event of the project to `onEvent`, in order; returns the way to stop. */
  subscribe(projectId: string, onEvent: (event: LiveEvent) => void): () => void;
  /** Closes the socket for good. */
  close(): void;
};

const OPEN = 1;
const LONGEST_DELAY_MS = 30_000;

type Subscription = {
  handlers: Set<(event: LiveEvent) => void>;
  /** The newest sequence number seen for the project, once there is one. */
  seq?: number;
};

/**
 * The reviewer's connection to the live updates of a workspace
 * (`/ws/w/<workspace>`). It subscribes to the projects that views ask for and,
 * after the socket drops, opens a new one and subscribes again with `after` set
 * to the newest sequence number it has for each project, so no event is lost.
 * The socket is opened when the first project is subscribed to.
 */
export function createLiveClient(options: LiveClientOptions): LiveClient {
  const { workspaceId, reconnectDelayMs = 1000 } = options;
  const subscriptions = new Map<string, Subscription>();
  let socket: SocketLike | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let failures = 0;
  let closed = false;

  const url = () => {
    const origin = new URL(options.origin ?? window.location.origin);
    const scheme = origin.protocol === "https:" ? "wss:" : "ws:";
    return `${scheme}//${origin.host}/ws/w/${workspaceId}`;
  };

  const send = (message: object) => socket?.send(JSON.stringify(message));

  const subscribeOnSocket = (projectId: string, subscription: Subscription) =>
    send({ type: "subscribe", project_id: projectId, after: subscription.seq });

  const scheduleReconnect = () => {
    if (closed || timer !== undefined || subscriptions.size === 0) return;
    const delay = Math.min(reconnectDelayMs * 2 ** failures, LONGEST_DELAY_MS);
    failures += 1;
    timer = setTimeout(() => {
      timer = undefined;
      connect();
    }, delay);
  };

  const receive = (data: unknown) => {
    let message: unknown;
    try {
      message = JSON.parse(String(data));
    } catch {
      return;
    }
    const parsed = LiveServerMessageSchema.safeParse(message);
    if (!parsed.success) return;
    const live = parsed.data;
    if (live.type === "subscribed") {
      const subscription = subscriptions.get(live.project_id);
      // With nothing seen yet, the project's newest number is where to resume.
      if (subscription && subscription.seq === undefined) subscription.seq = live.last_seq;
    } else if (live.type === "event") {
      const subscription = subscriptions.get(live.project_id);
      if (!subscription || (subscription.seq !== undefined && live.seq <= subscription.seq)) return;
      subscription.seq = live.seq;
      for (const handler of [...subscription.handlers]) {
        try {
          handler(live);
        } catch (error) {
          console.error("live event handler failed", error);
        }
      }
    }
  };

  const connect = () => {
    if (closed || socket) return;
    let opened: SocketLike;
    try {
      opened = new (options.WebSocket ?? WebSocket)(url());
    } catch {
      scheduleReconnect();
      return;
    }
    socket = opened;
    opened.addEventListener("open", () => {
      if (socket !== opened) return;
      failures = 0;
      for (const [projectId, subscription] of subscriptions) subscribeOnSocket(projectId, subscription);
    });
    opened.addEventListener("message", (event) => {
      if (socket === opened) receive(event.data);
    });
    opened.addEventListener("close", () => {
      if (socket !== opened) return;
      socket = null;
      scheduleReconnect();
    });
  };

  return {
    subscribe(projectId, onEvent) {
      let subscription = subscriptions.get(projectId);
      if (!subscription) {
        subscription = { handlers: new Set() };
        subscriptions.set(projectId, subscription);
        if (socket?.readyState === OPEN) subscribeOnSocket(projectId, subscription);
      }
      subscription.handlers.add(onEvent);
      connect();
      return () => {
        const current = subscriptions.get(projectId);
        current?.handlers.delete(onEvent);
        // The server has no unsubscribe; the project's events are dropped here.
        if (current && current.handlers.size === 0) subscriptions.delete(projectId);
      };
    },
    close() {
      closed = true;
      clearTimeout(timer);
      timer = undefined;
      const last = socket;
      socket = null;
      last?.close();
    },
  };
}
