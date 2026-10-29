import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { createFileRoute } from "@tanstack/react-router";
import {
  createDocComment,
  loadDoc,
  loadDocComments,
  loadHeadBoard,
  loadHeadHtml,
  loadHeadOutline,
} from "@/api/docs";
import type { CommentBody, DocBody } from "@/api/generated";
import {
  createDesignRenderer,
  type DesignRenderer,
  type Inspection,
  type Layer,
  type Property,
} from "@/design/renderer";
import { createAssetLoader } from "@/design/assets";
import { BoardCache, findPageHolding, showBoardwise } from "@/design/boards";
import {
  FileText,
  Files,
  Layers as LayersIcon,
  MessageSquare,
  MousePointerClick,
  PenTool,
  Send,
  SlidersHorizontal,
} from "lucide-react";
import { AgentChip } from "@/components/agent-chip";
import { Button } from "@/components/ui/button";
import {
  FloatingAlert,
  FloatingAlerts,
  PaneEmpty,
  PaneLabel,
  PaneSection,
  ToolbarSelect,
  ViewerToolbar,
} from "@/components/viewer/chrome";
import { LayerIcon, MonoValue, PropertyTable, TokenChip } from "@/components/viewer/design";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";
import { useReloading, type Reloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";
import { ViewerFrame } from "@/viewer/frame";
import { blockIds } from "@/viewer/html";

// The doc viewer is where the reviewer reads a doc as agents have it now and
// comments on its nodes. It reads the doc's head and reads it again when
// `doc.changed` says an agent wrote to the doc; there is no version to pick
// and no draft. The view keeps its place: the selected node, an HTML doc's
// scroll position (the frame draws the new HTML and puts its scroll back) and
// a design's pan and zoom (only the boards whose chunk changed are read again).
//
// An HTML doc is shown in a sandboxed frame (src/viewer/document.ts) that the
// page talks to by postMessage only: the frame tells which block was clicked,
// and the page tells it what to show, which block is selected and where the
// pins are.
//
// A design doc is shown by the design viewer instead (further down): the
// renderer's canvas with a page list, a layers panel, an inspect panel for the
// selected node, a switch between the design system's token modes and comment
// pins on the canvas.
//
// The `board` and `node` search parameters say where in the doc to be: the
// board that holds the selection (a design shows the page of that board) and
// the selected node address. They are read when the doc opens, and kept up to
// date with a history replace whenever the board or the selection changes.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/$docId")({
  component: DocPage,
  validateSearch: (search: Record<string, unknown>): { board?: string; node?: string } => ({
    board: typeof search.board === "string" && search.board ? search.board : undefined,
    node: typeof search.node === "string" && search.node ? search.node : undefined,
  }),
});

const TEXTAREA_CLASS =
  "block min-h-20 w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-[13px] leading-relaxed outline-none placeholder:text-muted-foreground";

/** No claims: a head has no drafts to hold them. */
const NO_CLAIMS: never[] = [];

function holderOfComment(author: Record<string, unknown>): string {
  return author.kind === "human" ? "Reviewer" : `${String(author.run)}/${String(author.role)}`;
}

/** Where the address bar says the reviewer is in the doc. */
type Place = { board?: string; node?: string };

function DocPage() {
  const { workspaceId, projectId, docId } = Route.useParams();
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  const doc = useReloading(useCallback(() => loadDoc(workspaceId, projectId, docId), [workspaceId, projectId, docId]));
  const current = useRef({ search, navigate });
  useEffect(() => {
    current.current = { search, navigate };
  });
  // Puts the place in the address bar, replacing the entry, so that it is always a link to where the reviewer is.
  const place = useCallback((next: Place) => {
    const { search: now, navigate: go } = current.current;
    if (next.board === now.board && next.node === now.node) return;
    void go({ search: next, replace: true });
  }, []);
  if (doc.value?.kind === "design") {
    return (
      <DesignViewer
        workspaceId={workspaceId}
        projectId={projectId}
        docId={docId}
        doc={doc}
        board={search.board}
        node={search.node}
        onPlace={place}
      />
    );
  }
  if (!doc.value) {
    return (
      <main className="flex min-h-full flex-col">
        <ViewerToolbar workspaceId={workspaceId} projectId={projectId} doc={null} />
        <div className="relative flex-1 bg-canvas">
          <FloatingAlerts>
            {doc.failed && <FloatingAlert>The doc could not be loaded. Try again in a moment.</FloatingAlert>}
          </FloatingAlerts>
        </div>
      </main>
    );
  }
  return (
    <HtmlDocViewer
      workspaceId={workspaceId}
      projectId={projectId}
      docId={docId}
      doc={doc}
      node={search.node}
      onPlace={place}
    />
  );
}

function HtmlDocViewer({
  workspaceId,
  projectId,
  docId,
  doc,
  node,
  onPlace,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
  node: string | undefined;
  onPlace: (place: Place) => void;
}) {
  const head = useReloading(useCallback(() => loadHeadHtml(workspaceId, projectId, docId), [workspaceId, projectId, docId]));
  const comments = useReloading(
    useCallback(() => loadDocComments(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const [selected, setSelected] = useState<string | null>(node ?? null);
  // The block the frame scrolls into view: the one the address bar named.
  const [reveal, setReveal] = useState<string | null>(node ?? null);
  const selectedRef = useRef(selected);
  useEffect(() => {
    selectedRef.current = selected;
  });

  // A link to another block of the doc selects that block and scrolls to it.
  useEffect(() => {
    if (node && node !== selectedRef.current) {
      setSelected(node);
      setReveal(node);
    }
  }, [node]);

  const html = head.value;
  const present = useMemo(() => (html === null ? null : blockIds(html)), [html]);

  // A selection whose block an agent removed is cleared.
  useEffect(() => {
    if (selected && present && !present.has(selected)) setSelected(null);
  }, [selected, present]);

  // The address bar follows the selection once the doc is there to say whether the block is.
  useEffect(() => {
    if (present === null) return;
    onPlace({ node: selected ?? undefined });
  }, [selected, present, onPlace]);

  const commentsOnSelected = useMemo(
    () => (comments.value ?? []).filter((comment) => comment.node === selected),
    [comments.value, selected],
  );

  useProjectEvents(projectId, (event) => {
    const { doc_id: eventDoc } = event.data;
    if (typeof eventDoc === "string" && eventDoc !== docId) return;
    const name = event.event;
    if (name === "doc.changed") head.reload();
    else if (name.startsWith("comment.")) comments.reload();
    else if (name.startsWith("doc.")) doc.reload();
  });

  const pins = useMemo(() => {
    if (present === null) return [];
    const counts = new Map<string, number>();
    for (const comment of comments.value ?? []) {
      if (present.has(comment.node)) counts.set(comment.node, (counts.get(comment.node) ?? 0) + 1);
    }
    return Array.from(counts, ([node, count]) => ({ node, count }));
  }, [present, comments.value]);

  return (
    <main className="flex min-h-full flex-col lg:h-full lg:min-h-[560px]">
      <ViewerToolbar workspaceId={workspaceId} projectId={projectId} doc={doc.value} icon={<FileText />} />
      <div className="flex min-h-0 flex-1 flex-col lg:grid lg:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="relative flex min-h-0 flex-col bg-canvas px-3 py-4 sm:px-6 sm:py-6 lg:px-8">
          <FloatingAlerts>
            {(doc.failed || head.failed) && (
              <FloatingAlert>The doc could not be loaded. Try again in a moment.</FloatingAlert>
            )}
          </FloatingAlerts>
          {html === "" && (
            <div className="mx-auto mb-4 w-full max-w-[56rem]">
              <PaneEmpty icon={<FileText />}>This doc has no content yet.</PaneEmpty>
            </div>
          )}
          <div className="mx-auto flex h-[78vh] min-h-0 w-full max-w-[56rem] flex-col overflow-hidden rounded-xl bg-white shadow-panel ring-1 ring-black/[0.04] lg:h-auto lg:flex-1">
            <ViewerFrame
              workspaceId={workspaceId}
              html={html}
              pins={pins}
              selected={selected}
              claims={NO_CLAIMS}
              reveal={reveal}
              onSelect={setSelected}
              className="block min-h-0 w-full flex-1 bg-white"
            />
          </div>
        </div>
        <aside
          aria-label="Review"
          className="border-t bg-background lg:min-h-0 lg:overflow-y-auto lg:border-t-0 lg:border-l"
        >
          <CommentsPanel
            workspaceId={workspaceId}
            projectId={projectId}
            docId={docId}
            selected={selected}
            comments={commentsOnSelected}
            noun="block"
            onPosted={comments.reload}
          />
        </aside>
      </div>
    </main>
  );
}

function CommentsPanel({
  workspaceId,
  projectId,
  docId,
  selected,
  comments,
  noun,
  onPosted,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  selected: string | null;
  comments: CommentBody[];
  /** What a comment is on: a block of a doc, a node of a design. */
  noun: string;
  onPosted: () => void;
}) {
  const [text, setText] = useState("");
  const [posting, setPosting] = useState(false);
  const [failed, setFailed] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const said = text.trim();
    if (!selected || !said || posting) return;
    setPosting(true);
    setFailed(false);
    try {
      await createDocComment(workspaceId, projectId, docId, { node: selected, text: said });
      setText("");
      onPosted();
    } catch {
      setFailed(true);
    } finally {
      setPosting(false);
    }
  }

  return (
    <PaneSection
      id="comments"
      title="Comments"
      aside={
        selected ? (
          <span className="max-w-32 truncate font-mono text-[10.5px]" title={selected}>
            {selected}
          </span>
        ) : undefined
      }
    >
      {!selected && (
        <PaneEmpty icon={<MousePointerClick />}>Select a {noun} to see its comments.</PaneEmpty>
      )}
      {selected && comments.length === 0 && (
        <PaneEmpty icon={<MessageSquare />}>No comments on this {noun} yet.</PaneEmpty>
      )}
      <ul className="space-y-2 empty:hidden">
        {comments.map((comment) => (
          <li key={comment.id} className="space-y-2 rounded-lg border bg-card p-3 text-[13px]">
            <div className="flex items-center justify-between gap-2">
              <AgentChip name={holderOfComment(comment.author)} />
              <div className="flex shrink-0 items-center gap-1.5">
                <span className="text-[11px] text-muted-foreground">{timeAgo(comment.created_at)}</span>
                {comment.waiting_on_you && (
                  <span className="rounded-full bg-signal-soft px-1.5 text-[11px] font-medium">Waiting on you</span>
                )}
              </div>
            </div>
            <p className="leading-relaxed break-words whitespace-pre-wrap">{comment.text}</p>
            {comment.replies.length > 0 && (
              <ul className="space-y-2 border-l-2 pl-2.5">
                {comment.replies.map((reply) => (
                  <li key={reply.id} className="space-y-0.5">
                    <AgentChip name={holderOfComment(reply.author)} />
                    <p className="leading-relaxed break-words whitespace-pre-wrap text-muted-foreground">
                      {reply.text}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </li>
        ))}
      </ul>
      {selected && (
        <form
          onSubmit={submit}
          className="mt-3 overflow-hidden rounded-lg border bg-background shadow-xs transition-shadow focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/20"
        >
          <label htmlFor="new-comment" className="sr-only">
            Comment
          </label>
          <textarea
            id="new-comment"
            className={TEXTAREA_CLASS}
            placeholder={`Comment on this ${noun}…`}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
          {failed && (
            <p role="alert" className="px-3 pb-1 text-[12px] text-destructive">
              The comment could not be saved.
            </p>
          )}
          <div className="flex items-center justify-end gap-2 px-2 pb-2">
            <Button type="submit" size="sm" className="h-7 gap-1.5 px-2.5 text-[12px]" disabled={posting || !text.trim()}>
              <Send aria-hidden className="size-3" />
              Comment
            </Button>
          </div>
        </form>
      )}
    </PaneSection>
  );
}

// The design viewer ------------------------------------------------------------------------

/** A layer and its depth, in the order the tree lists them. */
type Row = { layer: Layer; level: number };

function rowsOf(layers: Layer[], level = 1): Row[] {
  return layers.flatMap((layer) => [{ layer, level }, ...rowsOf(layer.children, level + 1)]);
}

/** The modes of the design system's token tree, in order. */
function modesOf(tokens: Record<string, unknown> | null | undefined): string[] {
  const extensions = tokens?.$extensions as { omnistate?: { modes?: unknown } } | undefined;
  const modes = extensions?.omnistate?.modes;
  return Array.isArray(modes) ? modes.filter((mode): mode is string => typeof mode === "string") : [];
}

/** A property value as the inspect panel writes it. */
function showValue(value: unknown): string {
  return typeof value === "string" ? value : JSON.stringify(value);
}

const NUMBER_FORMAT = new Intl.NumberFormat("en", { maximumFractionDigits: 2 });

function PropertyCells({ property, laidOut }: { property: Property; laidOut?: number }) {
  return (
    <span className="flex min-w-0 flex-col gap-1">
      <span className="flex min-w-0 flex-wrap items-baseline gap-x-1.5">
        <MonoValue text={showValue(property.value)} />
        {laidOut !== undefined && (
          <span className="font-mono text-[11px] text-muted-foreground"> ({NUMBER_FORMAT.format(laidOut)} px)</span>
        )}
      </span>
      {property.tokens.map((used) => (
        <span key={used.token} className="flex min-w-0 flex-wrap items-center gap-1.5">
          <TokenChip name={used.token} />{" "}
          <MonoValue text={showValue(used.value)} className="text-[11px] text-muted-foreground" />
        </span>
      ))}
    </span>
  );
}

/** Every property of the selected node, with the tokens it uses and their values in the current mode. */
function InspectPanel({ inspection }: { inspection: Inspection | null }) {
  const group = (label: string, properties: Property[], laidOut?: (name: string) => number | undefined) =>
    properties.length > 0 && (
      <PropertyTable
        label={label}
        rows={properties.map((property) => ({
          name: property.name,
          cells: <PropertyCells property={property} laidOut={laidOut?.(property.name)} />,
        }))}
      />
    );
  const instance = inspection?.instance;
  return (
    <PaneSection id="inspect" title="Inspect" aside={inspection?.mode ? <span className="font-mono">{inspection.mode}</span> : undefined}>
      {!inspection && <PaneEmpty icon={<SlidersHorizontal />}>Select a layer to inspect it.</PaneEmpty>}
      {inspection && (
        <div className="space-y-4">
          <div className="flex min-w-0 items-center gap-2 rounded-lg bg-muted px-2.5 py-2">
            <LayerIcon type={inspection.type} className="text-muted-foreground" />
            <p className="min-w-0 flex-1 truncate text-[13px] font-medium">
              {inspection.name}{" "}
              <span className="font-mono text-[10.5px] font-normal text-muted-foreground">{inspection.type}</span>
            </p>
            <span className="shrink-0 font-mono text-[10.5px] text-muted-foreground tabular-nums">
              {NUMBER_FORMAT.format(inspection.box.width)} × {NUMBER_FORMAT.format(inspection.box.height)}
            </span>
          </div>
          {group("Layout", inspection.layout)}
          {group("Size", inspection.size, (name) =>
            name === "width" ? inspection.box.width : name === "height" ? inspection.box.height : undefined,
          )}
          {group("Style", inspection.style)}
          {group("Text", inspection.text)}
          {instance && (
            <PropertyTable
              label="Component"
              rows={[
                {
                  name: "component",
                  cells: (
                    <span className="inline-flex items-center gap-1.5 font-mono text-violet-700 dark:text-violet-300">
                      <LayerIcon type="component" className="size-3" />
                      {instance.componentName}
                    </span>
                  ),
                },
                ...instance.props.map((prop) => ({
                  name: prop.name,
                  cells: (
                    <span className="flex min-w-0 flex-wrap items-baseline gap-x-1.5">
                      <MonoValue text={showValue(prop.value)} />
                      <span className="text-[11px] text-muted-foreground">
                        {" "}
                        {prop.kind}
                        {prop.set ? "" : ", default"}
                      </span>
                    </span>
                  ),
                })),
              ]}
            />
          )}
          {instance && instance.overrides.length > 0 && (
            <PropertyTable
              label="Overrides"
              rows={instance.overrides.map((override) => ({
                name: override.name,
                cells: <MonoValue text={showValue(override.fields)} />,
              }))}
            />
          )}
        </div>
      )}
    </PaneSection>
  );
}

type JsonNode = Record<string, unknown>;

/** The node with the ID `id` among `nodes` and what is below them. */
function findNode(nodes: JsonNode[], id: string): JsonNode | null {
  for (const node of nodes) {
    if (node.id === id) return node;
    const below = Array.isArray(node.children) ? findNode(node.children as JsonNode[], id) : null;
    if (below) return below;
  }
  return null;
}


/** Whether the layer `layer`, or one below it, has the address `address`. */
function layerHolds(layer: Layer, address: string): boolean {
  return layer.address === address || layer.children.some((below) => layerHolds(below, address));
}

/** The pages and boards a design's head is made of, and the design system it resolves against. */
type DesignSource = { pages: JsonNode[]; components: JsonNode[]; tokens: Record<string, unknown> | null };

/** Whether `page` holds a node with the ID `id`. */
function holds(page: JsonNode, id: string): boolean {
  return findNode([page], id) !== null;
}


/**
 * Buttons over the canvas, one on each commented node, that sit on the node's
 * box and follow it as the view changes (zooming, panning, another page).
 */
function CommentPins({
  renderer,
  pins,
  selected,
  onPick,
}: {
  renderer: DesignRenderer | null;
  pins: { address: string; name: string; count: number }[];
  selected: string | null;
  onPick: (address: string) => void;
}) {
  const [, setView] = useState(0);
  useEffect(() => renderer?.onView(() => setView((count) => count + 1)), [renderer]);
  if (!renderer) return null;
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden">
      {pins.map((pin) => {
        let box: ReturnType<DesignRenderer["boxOf"]>;
        try {
          box = renderer.boxOf(pin.address);
        } catch {
          return null;
        }
        if (!box) return null;
        return (
          <button
            key={pin.address}
            type="button"
            aria-label={`${pin.count} ${pin.count === 1 ? "comment" : "comments"} on ${pin.name}`}
            aria-pressed={pin.address === selected}
            // The pin's centre is the top-right corner of its node's box.
            style={{ left: box.x + box.width, top: box.y }}
            className="pointer-events-auto absolute flex h-6 min-w-6 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full rounded-bl-[4px] bg-signal px-1.5 text-[11px] font-semibold text-signal-foreground tabular-nums shadow-[0_2px_6px_-1px_oklch(0.2_0.01_60/30%)] ring-2 ring-background transition-transform hover:scale-110 focus-visible:outline-none aria-pressed:scale-110 aria-pressed:ring-signal/35 aria-pressed:ring-4"
            onClick={() => onPick(pin.address)}
          >
            {pin.count}
          </button>
        );
      })}
    </div>
  );
}

/**
 * The reviewer's view of a design doc: its head on the renderer's canvas, a
 * list of its pages and a layers panel whose selection follows the canvas, and
 * the other way around. The head is read as an outline (its pages with their
 * boards as placeholders, the design system's tokens and components) and the
 * boards in view come one by one; when an agent writes to the design, the
 * outline is read again and only the boards whose chunk changed are read
 * again, while the zoom, the pan and the selection stay.
 *
 * Comments pin on node addresses: a button over the canvas on each commented
 * node, and a Comments panel for the selected node, in which a new comment
 * attaches to the selected address.
 */
function DesignViewer({
  workspaceId,
  projectId,
  docId,
  doc,
  board: boardParam,
  node,
  onPlace,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
  board: string | undefined;
  node: string | undefined;
  onPlace: (place: Place) => void;
}) {
  const outline = useReloading(
    useCallback(() => loadHeadOutline(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const comments = useReloading(
    useCallback(() => loadDocComments(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const [chosen, setChosen] = useState<string | null>(null);
  const [renderer, setRenderer] = useState<DesignRenderer | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [layers, setLayers] = useState<Layer[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  // The node and the board the address bar named, until they are shown.
  const [target, setTarget] = useState<string | null>(node ?? null);
  const [boardTarget, setBoardTarget] = useState<string | null>(boardParam ?? null);
  const boardHint = useRef<string | null>(boardParam ?? null);
  const [chosenMode, setChosenMode] = useState<string | null>(null);
  // Counts the times the renderer drew a page or another mode, so that the inspect panel asks again.
  const [drawn, setDrawn] = useState(0);
  const canvas = useRef<HTMLCanvasElement>(null);

  const source: DesignSource | null = useMemo(
    () =>
      outline.value
        ? {
            pages: outline.value.pages as JsonNode[],
            components: outline.value.components as JsonNode[],
            tokens: outline.value.tokens,
          }
        : null,
    [outline.value],
  );

  // The boards read of the head, each with the chunk it was read at.
  const cache = useMemo(
    () => new BoardCache((board) => loadHeadBoard(workspaceId, projectId, docId, board)),
    [workspaceId, projectId, docId],
  );
  useEffect(() => {
    if (outline.value) cache.update(new Map(outline.value.boards.map((entry) => [entry.id, entry.chunk])));
  }, [cache, outline.value]);

  const modes = useMemo(() => modesOf(source?.tokens), [source]);
  const mode = chosenMode !== null && modes.includes(chosenMode) ? chosenMode : (modes[0] ?? null);
  const modeRef = useRef(mode);
  useEffect(() => {
    modeRef.current = mode;
  });
  const selectedRef = useRef(selected);
  useEffect(() => {
    selectedRef.current = selected;
  });

  const pages = useMemo(() => source?.pages ?? [], [source]);
  const page = useMemo(() => pages.find((candidate) => candidate.id === chosen) ?? pages[0] ?? null, [pages, chosen]);
  const system = useMemo(
    () => (source ? { ...source.tokens, $components: source.components } : null),
    [source],
  );

  useEffect(() => {
    const element = canvas.current;
    if (!element) return;
    let cancelled = false;
    let made: DesignRenderer | null = null;
    createDesignRenderer(element).then(
      (created) => {
        if (cancelled) {
          created.destroy();
          return;
        }
        made = created;
        setRenderer(created);
      },
      (error: unknown) => {
        if (!cancelled) setFailure(error instanceof Error ? error.message : "The renderer could not start.");
      },
    );
    return () => {
      cancelled = true;
      made?.destroy();
      setRenderer(null);
    };
  }, []);

  // A node selected on the canvas is the selected layer.
  useEffect(() => renderer?.onSelect(setSelected), [renderer]);

  const assets = useMemo(() => (renderer ? createAssetLoader(renderer, workspaceId) : null), [renderer, workspaceId]);

  useEffect(() => {
    if (!renderer || !assets || !page || !system) return;
    let cancelled = false;
    let stopBoards: (() => void) | null = null;
    // What went wrong so far: an image or a board that could not be loaded is
    // named in the error line, and the rest still draws.
    const problems: string[] = [];
    const report = () => setFailure(problems.length > 0 ? problems.join(" ") : null);
    const shown = () => {
      // A page is shown in the first mode, so the chosen mode is set again.
      if (modeRef.current && modeRef.current !== modes[0]) renderer.setMode(modeRef.current);
      const shownLayers = renderer.layers();
      // The selection stays while its node is still there, as when an agent changes the design.
      const kept = selectedRef.current;
      const stays = kept !== null && rowsOf(shownLayers).some((row) => row.layer.address === kept);
      if (stays) renderer.select(kept);
      setDrawn((count) => count + 1);
      setLayers(shownLayers);
      setSelected(stays ? kept : null);
      report();
    };
    const failed = (error: unknown) => {
      problems.push(error instanceof Error ? error.message : "The design could not be drawn.");
      report();
    };
    // The page is shown once its images and fonts are with the renderer.
    void assets.load(page, system).then((loaded) => {
      if (cancelled) return;
      problems.push(...loaded);
      try {
        // The page shows its boards as placeholders, and reads those in view.
        stopBoards = showBoardwise({
          renderer,
          page,
          system,
          cache,
          prepare: async (board) => {
            problems.push(...(await assets.load(board, system)));
          },
          onShown: shown,
          onFailure: (message) => {
            problems.push(message);
            report();
          },
        });
      } catch (error) {
        failed(error);
      }
    });
    return () => {
      cancelled = true;
      stopBoards?.();
    };
  }, [renderer, assets, page, system, modes, cache]);

  // A link to a node: its page is shown, then it is selected.
  useEffect(() => {
    if (node && node !== selectedRef.current) setTarget(node);
  }, [node]);
  // A link to a board shows the page that holds it; a board that is not there leaves the first page.
  useEffect(() => {
    if (!boardTarget || pages.length === 0) return;
    const holder = pages.find((candidate) =>
      (Array.isArray(candidate.children) ? (candidate.children as JsonNode[]) : []).some(
        (entry) => entry.id === boardTarget,
      ),
    );
    if (holder) setChosen(String(holder.id));
    setBoardTarget(null);
  }, [boardTarget, pages]);
  useEffect(() => {
    if (!target) return;
    const holder = pages.find((candidate) => holds(candidate, target.split("/")[0]));
    if (holder && holder !== page) setChosen(String(holder.id));
  }, [target, pages, page]);
  // A node inside a board is on the page whose board holds it, which the boards say: they are read until one does.
  useEffect(() => {
    if (!target || !assets || !system || pages.length === 0) return;
    const id = target.split("/")[0];
    if (pages.some((candidate) => holds(candidate, id))) return;
    let cancelled = false;
    void findPageHolding(
      cache,
      pages,
      id,
      async (board) => {
        await assets.load(board, system);
      },
      () => cancelled,
      boardHint.current,
    ).then((holder) => {
      if (cancelled) return;
      // No page holds the node: the board is shown with nothing selected.
      if (holder && holder !== page) setChosen(String(holder.id));
      else if (!holder) setTarget(null);
    });
    return () => {
      cancelled = true;
    };
  }, [target, assets, system, pages, page, cache]);
  useEffect(() => {
    if (!renderer || !target || !rowsOf(layers).some((row) => row.layer.address === target)) return;
    setSelected(target);
    renderer.select(target);
    setTarget(null);
  }, [renderer, target, layers]);

  // The address bar follows the page and the selection, once the ones it named are shown.
  useEffect(() => {
    if (target !== null || boardTarget !== null || !page || drawn === 0) return;
    const boards = Array.isArray(page.children) ? (page.children as JsonNode[]) : [];
    const holder = selected ? layers.find((layer) => layerHolds(layer, selected)) : undefined;
    const board = holder?.address ?? (boards[0] ? String(boards[0].id) : undefined);
    onPlace({ board, node: selected ?? undefined });
  }, [target, boardTarget, page, layers, selected, drawn, onPlace]);

  useProjectEvents(projectId, (event) => {
    const { doc_id: eventDoc } = event.data;
    if (typeof eventDoc === "string" && eventDoc !== docId) {
      // The design system is another doc, and a design resolves against it.
      return;
    }
    const name = event.event;
    if (name === "doc.changed") outline.reload();
    else if (name.startsWith("comment.")) comments.reload();
    else if (name.startsWith("doc.")) doc.reload();
    else if (name === "tokens.imported" || name === "components.imported") outline.reload();
  });

  function choose(address: string | null) {
    setSelected(address);
    renderer?.select(address);
  }

  function chooseMode(next: string) {
    try {
      renderer?.setMode(next);
      setFailure(null);
    } catch (error) {
      setFailure(error instanceof Error ? error.message : "The design could not be drawn in that mode.");
      return;
    }
    setChosenMode(next);
    setDrawn((count) => count + 1);
  }

  const rows = useMemo(() => rowsOf(layers), [layers]);
  const inspection = useMemo(() => {
    // `drawn` says the renderer drew again, which changes what it answers.
    if (!renderer || !selected || drawn < 0) return null;
    try {
      return renderer.inspect(selected);
    } catch {
      return null;
    }
  }, [renderer, selected, drawn]);

  const commentsOnSelected = useMemo(
    () => (comments.value ?? []).filter((comment) => comment.node === selected),
    [comments.value, selected],
  );

  // One pin on each node of the page shown that has comments.
  const pins = useMemo(() => {
    const names = new Map(rows.map(({ layer }) => [layer.address, layer.name]));
    const counts = new Map<string, number>();
    for (const comment of comments.value ?? []) {
      if (names.has(comment.node)) counts.set(comment.node, (counts.get(comment.node) ?? 0) + 1);
    }
    return Array.from(counts, ([address, count]) => ({ address, count, name: names.get(address) ?? address }));
  }, [rows, comments.value]);

  return (
    <main className="flex min-h-full flex-col lg:h-full lg:min-h-[560px]">
      <ViewerToolbar
        workspaceId={workspaceId}
        projectId={projectId}
        doc={doc.value}
        icon={<PenTool />}
      >
        {doc.value && mode !== null && (
          <ToolbarSelect
            id="mode-switch"
            label="Mode"
            className="w-full sm:w-40"
            value={mode}
            onChange={(event) => chooseMode(event.target.value)}
          >
            {modes.map((listed) => (
              <option key={listed} value={listed}>
                {listed}
              </option>
            ))}
          </ToolbarSelect>
        )}
      </ViewerToolbar>
      <div className="flex min-h-0 flex-1 flex-col md:grid md:grid-cols-2 lg:grid-cols-[13.5rem_minmax(0,1fr)_17rem] xl:grid-cols-[15.5rem_minmax(0,1fr)_19rem]">
        <div
          className="relative order-1 h-[62vh] min-h-80 overflow-hidden bg-muted md:col-span-2 lg:order-2 lg:col-span-1 lg:h-auto lg:min-h-0"
          style={{
            backgroundImage: "radial-gradient(circle at 1px 1px, oklch(0.5 0.012 60 / 0.22) 1px, transparent 0)",
            backgroundSize: "16px 16px",
          }}
        >
          <canvas ref={canvas} aria-label="Design canvas" className="absolute inset-0 size-full touch-none" />
          <CommentPins renderer={renderer} pins={pins} selected={selected} onPick={choose} />
          {outline.value && pages.length === 0 && (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-6">
              <p className="rounded-xl bg-background/90 px-4 py-3 text-[13px] text-muted-foreground shadow-panel">
                This design has no pages yet.
              </p>
            </div>
          )}
          <FloatingAlerts>
            {doc.failed && <FloatingAlert>The doc could not be loaded. Try again in a moment.</FloatingAlert>}
            {outline.failed && (
              <FloatingAlert>The design could not be loaded. Try again in a moment.</FloatingAlert>
            )}
            {failure && <FloatingAlert>{failure}</FloatingAlert>}
          </FloatingAlerts>
        </div>
        <aside
          aria-label="Pages and layers"
          className="order-2 flex flex-col border-t bg-background lg:order-1 lg:min-h-0 lg:border-t-0 lg:border-r"
        >
          <nav aria-label="Pages" className="shrink-0 border-b lg:max-h-[40%] lg:overflow-y-auto">
            <PaneLabel title="Pages" aside={pages.length > 0 ? pages.length : undefined} />
            <ul className="space-y-px px-1.5 pb-2">
              {pages.map((candidate) => (
                <li key={String(candidate.id)}>
                  <button
                    type="button"
                    className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px] text-muted-foreground transition-colors outline-none hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/40 aria-[current=page]:bg-accent aria-[current=page]:font-medium aria-[current=page]:text-foreground"
                    aria-current={candidate === page ? "page" : undefined}
                    onClick={() => setChosen(String(candidate.id))}
                  >
                    <Files aria-hidden className="size-3.5 shrink-0 opacity-60" />
                    <span className="truncate">{String(candidate.name ?? candidate.id)}</span>
                  </button>
                </li>
              ))}
            </ul>
          </nav>
          <section aria-labelledby="layers" className="flex min-h-0 flex-1 flex-col">
            <PaneLabel id="layers" title="Layers" aside={rows.length > 0 ? rows.length : undefined} />
            <div
              role="tree"
              aria-label="Layers of the page"
              className="max-h-96 min-h-0 flex-1 overflow-auto px-1.5 pb-3 text-[12.5px] lg:max-h-none"
            >
              {rows.length === 0 && (
                <div className="px-1.5">
                  <PaneEmpty icon={<LayersIcon />}>The page's layers show here once it is drawn.</PaneEmpty>
                </div>
              )}
              {rows.map(({ layer, level }) => {
                const inside = layer.address.includes("/");
                const component = layer.type === "instance" || layer.type === "component";
                return (
                  <div
                    key={layer.address}
                    role="treeitem"
                    tabIndex={0}
                    aria-level={level}
                    aria-selected={layer.address === selected}
                    style={{ paddingLeft: `${0.5 + (level - 1) * 0.875}rem` }}
                    className={cn(
                      "group relative flex h-7 cursor-pointer items-center gap-1.5 rounded-md pr-2 outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/40 aria-selected:bg-signal-soft aria-selected:text-signal",
                      level === 1 && "font-medium",
                      inside && "text-violet-700 dark:text-violet-300",
                    )}
                    onClick={() => choose(layer.address)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        choose(layer.address);
                      }
                    }}
                  >
                    {Array.from({ length: level - 1 }, (_, guide) => (
                      <span
                        key={guide}
                        aria-hidden
                        className="absolute inset-y-0 w-px bg-border group-aria-selected:bg-signal/20"
                        style={{ left: `${0.5 + guide * 0.875 + 0.4}rem` }}
                      />
                    ))}
                    <LayerIcon
                      type={layer.type}
                      className={cn(
                        "text-muted-foreground group-aria-selected:text-signal",
                        (component || inside) && "text-violet-600 dark:text-violet-300",
                      )}
                    />
                    <span className={cn("min-w-0 flex-1 truncate", inside && "italic")}>{layer.name}</span>{" "}
                    <span className="shrink-0 font-mono text-[10px] text-muted-foreground/70 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100 group-aria-selected:text-signal/70 group-aria-selected:opacity-100">
                      {layer.type}
                    </span>
                  </div>
                );
              })}
            </div>
          </section>
        </aside>
        <aside
          aria-label="Review"
          className="order-3 border-t bg-background md:border-l lg:min-h-0 lg:overflow-y-auto lg:border-t-0"
        >
          <CommentsPanel
            workspaceId={workspaceId}
            projectId={projectId}
            docId={docId}
            selected={selected}
            comments={commentsOnSelected}
            noun="node"
            onPosted={comments.reload}
          />
          <InspectPanel inspection={inspection} />
        </aside>
      </div>
    </main>
  );
}

