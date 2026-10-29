// Showing a big design board by board. A design's head is read as its
// outline first (pages with their boards as placeholders, no board content);
// the renderer is shown a page with every board as a placeholder, and the
// boards that its view shows are read one by one, `READS_AT_ONCE` at a time,
// and put in place of their placeholders. A board is read once for a version,
// however often its page is shown, and the renderer keeps its view when the
// same page is shown again with more boards loaded.

import type { DesignRenderer } from "@/design/renderer";

type Json = Record<string, unknown>;

/** How many boards are read at the same time. */
const READS_AT_ONCE = 6;

/**
 * How many of the boards in view are read when more than that are in view,
 * as when the whole design is zoomed out to fit: the first ones in page order.
 * The rest stay placeholders until the reviewer zooms in on them.
 */
const MOST_IN_VIEW = 48;

/** How long to wait for more boards to arrive before showing the page again. */
const SHOW_AFTER_MS = 16;

/**
 * The boards of one design that have been read, and the reads under way. A
 * design's head changes: `update` tells the cache the chunk each board has now,
 * and a board whose chunk changed is read again while the old one stays in
 * place, so that a page does not flicker or lose its selection. A board whose
 * chunk is the same is never read again.
 */
export class BoardCache {
  private readonly boards = new Map<string, { board: Json; chunk: string | undefined }>();
  private readonly reading = new Map<string, Promise<Json>>();
  private readonly failed = new Set<string>();
  private readonly listeners = new Set<(id: string) => void>();
  /** The chunk each board has now; empty for a design that is not updated. */
  private chunks = new Map<string, string>();

  constructor(private readonly read: (boardId: string) => Promise<Json>) {}

  /** The board `id` once it has been read; an out-of-date board until the new one is read. */
  get(id: string): Json | undefined {
    return this.boards.get(id)?.board;
  }

  /** Whether the board `id` has to be read: it has not been, or its chunk changed since. */
  needsRead(id: string): boolean {
    const held = this.boards.get(id);
    return !held || (this.chunks.has(id) && held.chunk !== this.chunks.get(id));
  }

  /** The chunks the design's boards have now, by board ID; boards no longer in it are forgotten. */
  update(chunks: ReadonlyMap<string, string>): void {
    this.chunks = new Map(chunks);
    for (const id of Array.from(this.boards.keys())) if (!chunks.has(id)) this.boards.delete(id);
    for (const id of Array.from(this.failed)) if (this.needsRead(id)) this.failed.delete(id);
  }

  /** How many reads are under way. */
  get busy(): number {
    return this.reading.size;
  }

  /** Calls `listener` with the ID of every board that is read from now on; returns the function that stops it. */
  subscribe(listener: (id: string) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Whether the board `id` is being read. */
  isReading(id: string): boolean {
    return this.reading.has(id);
  }

  /** Whether the board `id` could not be read. */
  hasFailed(id: string): boolean {
    return this.failed.has(id);
  }

  /**
   * Reads the board `id` once, and gives `prepare` the board before it counts
   * as read. A board that could not be read is not tried again.
   */
  fetch(id: string, prepare: (board: Json) => Promise<unknown>): Promise<Json> {
    const held = this.boards.get(id);
    if (held && !this.needsRead(id)) return Promise.resolve(held.board);
    const running = this.reading.get(id);
    if (running) return running;
    const chunk = this.chunks.get(id);
    const started = this.read(id)
      .then(async (board) => {
        await prepare(board);
        return board;
      })
      .then(
        (board) => {
          this.boards.set(id, { board, chunk });
          this.reading.delete(id);
          for (const listener of Array.from(this.listeners)) listener(id);
          return board;
        },
        (error: unknown) => {
          this.failed.add(id);
          this.reading.delete(id);
          throw error;
        },
      );
    this.reading.set(id, started);
    return started;
  }
}

/** The page with the boards in `cache` in place of their placeholders. */
export function pageWithBoards(page: Json, cache: BoardCache): Json {
  const boards = Array.isArray(page.children) ? (page.children as Json[]) : [];
  return { ...page, children: boards.map((board) => cache.get(String(board.id)) ?? board) };
}

/**
 * The fill of a board that is not loaded yet: a light neutral grey, so that a
 * placeholder is not taken for the design itself (a dark board fill would
 * make it look like a black design).
 */
export const PLACEHOLDER_FILL = "#e4e4e7";

/**
 * What the renderer draws for `board`, a board of the outline that is not
 * loaded yet: a box with the board's ID, name, place and size (the API gives a
 * board that hugs its content the size of that content) and its rounding, so
 * that its label and its room on the canvas are the board's, filled with
 * `PLACEHOLDER_FILL` instead of the board's own look. Its layout is `none`:
 * the renderer refuses a board without a layout, and a board with no children
 * needs no other.
 */
export function placeholderOf(board: Json): Json {
  const kept: Json = { layout: { mode: "none" }, fills: [{ type: "solid", color: PLACEHOLDER_FILL }] };
  for (const key of ["id", "type", "name", "x", "y", "width", "height", "visible", "radius"]) {
    if (key in board) kept[key] = board[key];
  }
  return kept;
}

/** The page as the renderer draws it: the boards in `cache` loaded, the others as placeholders. */
function pageToDraw(page: Json, cache: BoardCache): Json {
  const boards = Array.isArray(page.children) ? (page.children as Json[]) : [];
  return { ...page, children: boards.map((board) => cache.get(String(board.id)) ?? placeholderOf(board)) };
}

export type BoardwiseOptions = {
  renderer: DesignRenderer;
  /** The page as the version's outline has it: every board a placeholder. */
  page: Json;
  system: unknown;
  cache: BoardCache;
  /** Gives the renderer what a board draws with (its images) before the board is shown. */
  prepare: (board: Json) => Promise<unknown>;
  /** The page was shown to the renderer, with this many boards loaded. */
  onShown: () => void;
  /** A board could not be read or drawn. */
  onFailure: (message: string) => void;
};

/**
 * Shows `page` on the renderer and keeps loading the boards that its view
 * shows, until the returned function is called. The page is shown once now,
 * with the boards that the cache holds, and again whenever boards arrive.
 */
export function showBoardwise(options: BoardwiseOptions): () => void {
  const { renderer, page, system, cache, prepare, onShown, onFailure } = options;
  const boardIds = new Set((Array.isArray(page.children) ? (page.children as Json[]) : []).map((board) => String(board.id)));
  // The boards this page is waiting for.
  const waiting = new Set<string>();
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function show() {
    if (stopped) return;
    renderer.show(pageToDraw(page, cache), system);
    onShown();
  }

  function showSoon() {
    if (timer !== null) return;
    timer = setTimeout(() => {
      timer = null;
      try {
        show();
      } catch (error) {
        onFailure(error instanceof Error ? error.message : "The design could not be drawn.");
      }
    }, SHOW_AFTER_MS);
  }

  // A board read for another reason (a search for a node) is shown too.
  const unsubscribeBoards = cache.subscribe((id) => {
    if (boardIds.has(id)) showSoon();
  });

  // What went wrong asking the renderer for the boards in view, said once.
  let viewFailure: string | null = null;

  function refresh() {
    if (stopped) return;
    let visible: string[];
    try {
      visible = renderer.visibleBoards();
    } catch (error) {
      // No board can be read without it, so this is never kept quiet.
      const message = `The boards in view could not be found, so none is loaded${error instanceof Error && error.message ? ` (${error.message})` : ""}.`;
      if (message !== viewFailure) {
        viewFailure = message;
        onFailure(message);
      }
      return;
    }
    viewFailure = null;
    const wanted = visible.filter((id) => boardIds.has(id)).slice(0, MOST_IN_VIEW);
    for (const id of wanted) {
      if (!cache.needsRead(id) || cache.hasFailed(id) || waiting.has(id)) continue;
      // A read that another page of the version started is waited for, not counted again.
      if (!cache.isReading(id) && cache.busy >= READS_AT_ONCE) continue;
      waiting.add(id);
      cache.fetch(id, prepare).then(
        () => {
          waiting.delete(id);
          if (stopped) return;
          refresh();
        },
        (error: unknown) => {
          waiting.delete(id);
          if (stopped) return;
          onFailure(`The board ${id} could not be loaded${error instanceof Error && error.message ? ` (${error.message})` : ""}.`);
          refresh();
        },
      );
    }
  }

  show();
  const unsubscribe = renderer.onView(refresh);
  refresh();
  return () => {
    stopped = true;
    if (timer !== null) clearTimeout(timer);
    unsubscribe();
    unsubscribeBoards();
  };
}

/** Whether `node` is, or holds below it, a node with the ID `id`. */
function contains(node: Json, id: string): boolean {
  const pending: Json[] = [node];
  while (pending.length > 0) {
    const next = pending.pop() as Json;
    if (next.id === id) return true;
    if (Array.isArray(next.children)) pending.push(...(next.children as Json[]));
  }
  return false;
}

/** How many boards a search for a node reads at most. */
const MOST_SEARCHED = 120;

/**
 * The page whose boards hold the node `id`, found by reading the boards of
 * `pages` (an outline's pages) in order, a few at a time, until one holds it;
 * `null` when none of the first `MOST_SEARCHED` does. A page or a board of the
 * outline itself is found without a read. The boards read are in `cache`.
 * `firstBoard`, a board the node is expected to be in, is read before the rest.
 */
export async function findPageHolding(
  cache: BoardCache,
  pages: Json[],
  id: string,
  prepare: (board: Json) => Promise<unknown>,
  cancelled: () => boolean,
  firstBoard?: string | null,
): Promise<Json | null> {
  const listed = pages.flatMap((page) =>
    (Array.isArray(page.children) ? (page.children as Json[]) : []).map((board) => ({ page, id: String(board.id) })),
  );
  const boards = [...listed.filter((entry) => entry.id === firstBoard), ...listed.filter((entry) => entry.id !== firstBoard)];
  const own = pages.find((page) => page.id === id || (Array.isArray(page.children) && (page.children as Json[]).some((board) => board.id === id)));
  if (own) return own;
  const searched = boards.slice(0, MOST_SEARCHED);
  for (let start = 0; start < searched.length; start += READS_AT_ONCE) {
    const found = await Promise.all(
      searched.slice(start, start + READS_AT_ONCE).map(async (entry) => {
        try {
          return contains(await cache.fetch(entry.id, prepare), id) ? entry.page : null;
        } catch {
          return null;
        }
      }),
    );
    if (cancelled()) return null;
    const hit = found.find((page) => page !== null);
    if (hit) return hit;
  }
  return null;
}
