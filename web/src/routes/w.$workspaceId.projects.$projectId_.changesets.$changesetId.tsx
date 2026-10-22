import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { ApiError } from "@/api/client";
import { loadVersion, loadVersionHtml } from "@/api/docs";
import type { DecisionBody, RecordedConflict, ReviewedDoc, ReviewResponse, VersionBody } from "@/api/generated";
import { approveChangeset, loadConflicts, loadReview, rejectChangeset } from "@/api/review";
import { createAssetLoader } from "@/design/assets";
import { createDesignRenderer, type DesignRenderer } from "@/design/renderer";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { useReloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";
import { ViewerFrame, type BlockChange } from "@/viewer/frame";
import { blockText } from "@/viewer/html";

// The change set review is where the reviewer sees exactly what a change set
// changes and approves or rejects it in one place. It lists each doc the
// change set changes with a summary of its node changes. A design shows the
// base and the new version side by side on two canvases, with the added and
// changed nodes outlined on the new version and the removed nodes on the base;
// an HTML doc shows the version under review in the sandboxed viewer frame,
// whose added and changed blocks are marked. The linked decisions show their
// status, and Approve stays disabled, naming the first decision that is not
// approved, until every one is. A panel lists the conflicts of the change set's
// rebases and how each was resolved. The page is loaded again on every live
// event of the project.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/changesets/$changesetId")({
  component: ChangesetReview,
});

const TEXTAREA_CLASS =
  "min-h-24 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";
const SELECT_CLASS =
  "h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";

/** The classes of a highlight over a canvas, by the kind of change. */
const HIGHLIGHT_CLASS = {
  added: "border-green-600 bg-green-600/15",
  changed: "border-amber-500 bg-amber-500/15",
  removed: "border-red-600 bg-red-600/15",
};

function statusWords(status: string): string {
  return status.replace(/_/g, " ");
}

function ChangesetReview() {
  const { workspaceId, projectId, changesetId } = Route.useParams();
  const review = useReloading(
    useCallback(() => loadReview(workspaceId, projectId, changesetId), [workspaceId, projectId, changesetId]),
  );
  const conflicts = useReloading(
    useCallback(() => loadConflicts(workspaceId, projectId, changesetId), [workspaceId, projectId, changesetId]),
  );
  const reloadReview = review.reload;
  const reloadConflicts = conflicts.reload;
  useProjectEvents(projectId, () => {
    reloadReview();
    reloadConflicts();
  });
  const loaded = review.value;

  return (
    <main className="mx-auto max-w-7xl space-y-8 p-6">
      <Link
        to="/w/$workspaceId/projects/$projectId"
        params={{ workspaceId, projectId }}
        className="text-sm text-muted-foreground underline-offset-4 hover:underline"
      >
        Back to the project
      </Link>
      {review.failed && (
        <Alert variant="destructive">
          <AlertDescription>The change set could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {loaded && (
        <>
          <Header review={loaded} />
          <Decisions workspaceId={workspaceId} projectId={projectId} decisions={loaded.decisions} />
          <Verdict
            workspaceId={workspaceId}
            projectId={projectId}
            review={loaded}
            onChanged={reloadReview}
          />
          <Docs workspaceId={workspaceId} projectId={projectId} docs={loaded.docs} />
          <Conflicts docs={loaded.docs} conflicts={conflicts.value} failed={conflicts.failed} />
        </>
      )}
    </main>
  );
}

function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="space-y-3">
      <h2 id={id} className="text-lg font-semibold">
        {title}
      </h2>
      {children}
    </section>
  );
}

function Header({ review }: { review: ReviewResponse }) {
  const { changeset } = review;
  const rejection = [...changeset.history].reverse().find((entry) => entry.to_status === "rejected")?.reason;
  return (
    <header className="space-y-2">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-semibold">{changeset.title}</h1>
        <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
          {statusWords(changeset.status)}
        </span>
      </div>
      {changeset.description && <p className="text-sm text-muted-foreground">{changeset.description}</p>}
      {changeset.status === "rejected" && rejection && <p className="text-sm">Reason: {rejection}</p>}
    </header>
  );
}

// Decisions and the verdict -----------------------------------------------------------------

function Decisions({
  workspaceId,
  projectId,
  decisions,
}: {
  workspaceId: string;
  projectId: string;
  decisions: DecisionBody[];
}) {
  return (
    <Section id="decisions" title="Decisions">
      {decisions.length === 0 && <p className="text-sm text-muted-foreground">No decision is linked.</p>}
      <ul className="divide-y rounded-md border empty:hidden">
        {decisions.map((decision) => (
          <li key={decision.id} className="flex items-center justify-between gap-3 px-4 py-2 text-sm">
            <Link
              to="/w/$workspaceId/projects/$projectId/threads/$threadId"
              params={{ workspaceId, projectId, threadId: decision.thread_id }}
              className="underline-offset-4 hover:underline"
            >
              {decision.direction}
            </Link>
            <span className="rounded-full border px-2 py-0.5 text-xs text-muted-foreground">{decision.status}</span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

function Verdict({
  workspaceId,
  projectId,
  review,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  review: ReviewResponse;
  onChanged: () => void;
}) {
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const blockedId = useId();
  const reasonId = useId();
  const { changeset } = review;
  if (changeset.status !== "in_review") return null;

  const undecided = review.decisions.find((decision) => decision.status !== "approved");

  async function act(call: () => Promise<unknown>) {
    setBusy(true);
    setFailure(null);
    try {
      await call();
      setRejecting(false);
      setReason("");
      onChanged();
    } catch (error) {
      setFailure(
        error instanceof ApiError && error.code === "decision_not_approved"
          ? "A linked decision is not approved yet."
          : "The change set could not be changed. Reload and try again.",
      );
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-label="Verdict" className="space-y-3 rounded-md border p-4">
      {failure && (
        <Alert variant="destructive">
          <AlertDescription>{failure}</AlertDescription>
        </Alert>
      )}
      {rejecting && (
        <div className="space-y-1">
          <Label htmlFor={reasonId}>Reason</Label>
          <textarea
            id={reasonId}
            className={TEXTAREA_CLASS}
            value={reason}
            onChange={(event) => setReason(event.target.value)}
          />
        </div>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          disabled={busy || undecided !== undefined}
          aria-describedby={undecided ? blockedId : undefined}
          onClick={() => act(() => approveChangeset(workspaceId, projectId, changeset.id))}
        >
          Approve
        </Button>
        {rejecting ? (
          <>
            <Button
              variant="destructive"
              disabled={busy || !reason.trim()}
              onClick={() => act(() => rejectChangeset(workspaceId, projectId, changeset.id, reason.trim()))}
            >
              Reject change set
            </Button>
            <Button variant="ghost" onClick={() => setRejecting(false)}>
              Cancel
            </Button>
          </>
        ) : (
          <Button variant="outline" disabled={busy} onClick={() => setRejecting(true)}>
            Reject
          </Button>
        )}
      </div>
      {undecided && (
        <p id={blockedId} className="text-sm text-muted-foreground">
          Waiting for the decision to be approved: {undecided.direction}
        </p>
      )}
    </section>
  );
}

// The docs -------------------------------------------------------------------------------------

type Mark = { address: string; change: "added" | "changed" | "removed" };

/** The node changes of a doc as marks: the added, changed (and moved) nodes of the new version, and the removed nodes of the base. */
function marksOf(doc: ReviewedDoc): { current: Mark[]; base: Mark[] } {
  const { added, changed, moved, removed } = doc.changes;
  const current: Mark[] = [];
  const seen = new Set<string>();
  const take = (nodes: { id: string; type: string }[], change: Mark["change"], into: Mark[]) => {
    for (const node of nodes) {
      // A page is a name for boards; only boards and the nodes below them are drawn.
      if (node.type === "page" || seen.has(`${change}:${node.id}`)) continue;
      seen.add(`${change}:${node.id}`);
      into.push({ address: node.id, change });
    }
  };
  take(added, "added", current);
  take(changed, "changed", current);
  take(
    moved.filter((node) => !changed.some((other) => other.id === node.id)),
    "changed",
    current,
  );
  const base: Mark[] = [];
  take(removed, "removed", base);
  return { current, base };
}

function Docs({ workspaceId, projectId, docs }: { workspaceId: string; projectId: string; docs: ReviewedDoc[] }) {
  return (
    <Section id="docs" title="Docs">
      <p className="text-sm text-muted-foreground">
        Green marks what the change set adds, amber what it changes and red what it removes.
      </p>
      {docs.length === 0 && <p className="text-sm text-muted-foreground">The change set has not saved a version of any doc.</p>}
      <ul className="space-y-6 empty:hidden">
        {docs.map((doc) => (
          <li key={doc.doc_id} className="space-y-3 rounded-md border p-4">
            <div className="space-y-1">
              <h3 className="text-base font-medium">{doc.title}</h3>
              <Summary doc={doc} />
            </div>
            {doc.removed ? (
              <p className="text-sm text-muted-foreground">The change set removes this doc.</p>
            ) : doc.kind === "design" ? (
              <DesignChanges workspaceId={workspaceId} projectId={projectId} doc={doc} />
            ) : doc.kind === "html" ? (
              <HtmlChanges workspaceId={workspaceId} projectId={projectId} doc={doc} />
            ) : null}
          </li>
        ))}
      </ul>
    </Section>
  );
}

function Summary({ doc }: { doc: ReviewedDoc }) {
  const { added, changed, moved, removed } = doc.changes;
  const parts = [`${added.length} added`, `${changed.length} changed`, `${removed.length} removed`];
  if (moved.length > 0) parts.push(`${moved.length} moved`);
  return (
    <p className="text-sm text-muted-foreground">
      {parts.join(", ")}
      {doc.base_version_id === null && !doc.removed ? " (a new doc)" : ""}
    </p>
  );
}

type JsonNode = Record<string, unknown>;

/** Whether the node `root` is, or holds below it, a node with the ID `id`. */
function holds(root: JsonNode, id: string): boolean {
  if (root.id === id) return true;
  return Array.isArray(root.children) && (root.children as JsonNode[]).some((child) => holds(child, id));
}

/** Loads a saved version once; a saved version never changes. */
function useVersion(workspaceId: string, projectId: string, versionId: string | null) {
  const [version, setVersion] = useState<{ id: string; body: VersionBody } | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!versionId) return;
    let cancelled = false;
    setFailed(false);
    loadVersion(workspaceId, projectId, versionId).then(
      (body) => {
        if (!cancelled) setVersion({ id: versionId, body });
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, projectId, versionId]);
  return { body: version && version.id === versionId ? version.body : null, failed };
}

function DesignChanges({ workspaceId, projectId, doc }: { workspaceId: string; projectId: string; doc: ReviewedDoc }) {
  const base = useVersion(workspaceId, projectId, doc.base_version_id ?? null);
  const current = useVersion(workspaceId, projectId, doc.version_id);
  const marks = useMemo(() => marksOf(doc), [doc]);
  const [chosen, setChosen] = useState<string | null>(null);

  const currentPages = useMemo(() => current.body?.pages ?? [], [current.body]);
  const basePages = useMemo(() => base.body?.pages ?? [], [base.body]);
  const pageList = useMemo(() => {
    const listed = new Map<string, string>();
    for (const page of [...currentPages, ...basePages]) {
      if (!listed.has(String(page.id))) listed.set(String(page.id), String(page.name ?? page.id));
    }
    return Array.from(listed, ([id, name]) => ({ id, name }));
  }, [currentPages, basePages]);

  // At first the page that holds the first change: of the new version, else of the base.
  const first = useMemo(() => {
    const inCurrent = currentPages.find((page) => marks.current.some((mark) => holds(page, mark.address)));
    const inBase = basePages.find((page) => marks.base.some((mark) => holds(page, mark.address)));
    return String((inCurrent ?? inBase ?? currentPages[0] ?? basePages[0])?.id ?? "");
  }, [currentPages, basePages, marks]);
  const pageId = chosen !== null && pageList.some((page) => page.id === chosen) ? chosen : first;
  const selectId = useId();

  return (
    <div className="space-y-3">
      {(base.failed || current.failed) && (
        <Alert variant="destructive">
          <AlertDescription>The design could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {pageList.length > 1 && (
        <div className="w-64 space-y-1">
          <Label htmlFor={selectId}>Page</Label>
          <select id={selectId} className={SELECT_CLASS} value={pageId} onChange={(event) => setChosen(event.target.value)}>
            {pageList.map((page) => (
              <option key={page.id} value={page.id}>
                {page.name}
              </option>
            ))}
          </select>
        </div>
      )}
      <div className="grid gap-4 lg:grid-cols-2">
        <DesignCanvas
          workspaceId={workspaceId}
          label="Base"
          empty={doc.base_version_id === null ? "The doc is new, so it has no base." : "Loading"}
          source={base.body}
          pageId={pageId}
          marks={marks.base}
        />
        <DesignCanvas
          workspaceId={workspaceId}
          label="New version"
          empty="Loading"
          source={current.body}
          pageId={pageId}
          marks={marks.current}
        />
      </div>
    </div>
  );
}

/** One side of a design change: a version drawn on its own canvas, with its changed nodes outlined. */
function DesignCanvas({
  workspaceId,
  label,
  empty,
  source,
  pageId,
  marks,
}: {
  workspaceId: string;
  label: string;
  empty: string;
  source: VersionBody | null;
  pageId: string;
  marks: Mark[];
}) {
  const headingId = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const [renderer, setRenderer] = useState<DesignRenderer | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  // Counts the times the renderer drew, so that the highlights are placed again.
  const [, setDrawn] = useState(0);

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
  useEffect(() => renderer?.onView(() => setDrawn((count) => count + 1)), [renderer]);

  const page = useMemo(() => (source?.pages ?? []).find((candidate) => String(candidate.id) === pageId) ?? null, [source, pageId]);
  const system = useMemo(() => (source ? { ...source.tokens, $components: source.components ?? [] } : null), [source]);
  const assets = useMemo(() => (renderer ? createAssetLoader(renderer, workspaceId) : null), [renderer, workspaceId]);
  useEffect(() => {
    if (!renderer || !assets || !page || !system) return;
    let cancelled = false;
    // The page is shown once its images and fonts are with the renderer. One
    // that fails to load is named in the canvas's message, and the rest still draws.
    void assets.load(page, system).then((problems) => {
      if (cancelled) return;
      try {
        renderer.show(page, system);
        setDrawn((count) => count + 1);
        setFailure(problems.length > 0 ? problems.join(" ") : null);
      } catch (error) {
        setFailure([...problems, error instanceof Error ? error.message : "The design could not be drawn."].join(" "));
      }
    });
    return () => {
      cancelled = true;
    };
  }, [renderer, assets, page, system]);

  const highlights = page && renderer ? marks.flatMap((mark) => highlightOf(renderer, mark)) : [];
  const message = failure ?? (source === null ? empty : page === null ? "This version has no such page." : null);

  return (
    <section aria-labelledby={headingId} className="space-y-2">
      <h4 id={headingId} className="text-sm font-medium">
        {label}
      </h4>
      <div className="relative h-[60vh] overflow-hidden rounded-md border bg-muted/30">
        <canvas ref={canvas} aria-label={`${label} canvas`} className="size-full touch-none" />
        <div className="pointer-events-none absolute inset-0 overflow-hidden">{highlights}</div>
        {message && (
          <p className="absolute inset-0 flex items-center justify-center bg-muted/60 p-4 text-center text-sm text-muted-foreground">
            {message}
          </p>
        )}
      </div>
    </section>
  );
}

/** The element that outlines a changed node on its box, none when the page shown has no such node. */
function highlightOf(renderer: DesignRenderer, mark: Mark): ReactNode[] {
  let box: ReturnType<DesignRenderer["boxOf"]>;
  try {
    box = renderer.boxOf(mark.address);
  } catch {
    return [];
  }
  if (!box) return [];
  return [
    <div
      key={`${mark.change}:${mark.address}`}
      data-change={mark.change}
      data-node={mark.address}
      style={{ left: box.x, top: box.y, width: box.width, height: box.height }}
      className={`absolute rounded-sm border-2 ${HIGHLIGHT_CLASS[mark.change]}`}
    />,
  ];
}

function HtmlChanges({ workspaceId, projectId, doc }: { workspaceId: string; projectId: string; doc: ReviewedDoc }) {
  const html = useReloading(
    useCallback(
      () => loadVersionHtml(workspaceId, projectId, doc.doc_id, doc.version_id),
      [workspaceId, projectId, doc.doc_id, doc.version_id],
    ),
  );
  const baseId = doc.base_version_id ?? null;
  const baseHtml = useReloading(
    useCallback(
      () => (baseId ? loadVersionHtml(workspaceId, projectId, doc.doc_id, baseId) : Promise.resolve(null)),
      [workspaceId, projectId, doc.doc_id, baseId],
    ),
  );
  const { added, changed, moved, removed } = doc.changes;
  // The blocks the new version has and the change set added or changed; a removed block is not in it.
  const changes = useMemo<BlockChange[]>(
    () => [
      ...added.map((node) => ({ node: node.id, change: "added" as const })),
      ...changed.map((node) => ({ node: node.id, change: "changed" as const })),
      ...moved
        .filter((node) => !changed.some((other) => other.id === node.id))
        .map((node) => ({ node: node.id, change: "changed" as const })),
    ],
    [added, changed, moved],
  );
  const gone = removed
    .map((node) => ({ node: node.id, text: baseHtml.value ? blockText(baseHtml.value, node.id) : null }))
    .filter((block) => block.text);

  return (
    <div className="space-y-3">
      {html.failed && (
        <Alert variant="destructive">
          <AlertDescription>The doc could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      <ViewerFrame
        workspaceId={workspaceId}
        html={html.value}
        pins={NO_PINS}
        selected={null}
        claims={NO_CLAIMS}
        changes={changes}
        onSelect={ignore}
        className="h-[50vh] w-full rounded-md border bg-white"
        title={`${doc.title}, the version under review`}
      />
      {gone.length > 0 && (
        <div className="space-y-1">
          <h4 className="text-sm font-medium">Removed blocks</h4>
          <ul className="list-disc pl-5 text-sm text-red-700 dark:text-red-400">
            {gone.map((block) => (
              <li key={block.node} className="line-through">
                {block.text}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

const NO_PINS: { node: string; count: number }[] = [];
const NO_CLAIMS: never[] = [];
function ignore() {}

// Conflicts ------------------------------------------------------------------------------------

function Conflicts({
  docs,
  conflicts,
  failed,
}: {
  docs: ReviewedDoc[];
  conflicts: RecordedConflict[] | null;
  failed: boolean;
}) {
  const titleOf = (docId: string) => docs.find((doc) => doc.doc_id === docId)?.title ?? docId;
  return (
    <Section id="conflicts" title="Conflicts">
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The conflicts could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {conflicts && conflicts.length === 0 && (
        <p className="text-sm text-muted-foreground">No rebase of this change set found a conflict.</p>
      )}
      <ul className="divide-y rounded-md border empty:hidden">
        {(conflicts ?? []).map((conflict) => (
          <li key={conflict.id} className="space-y-1 px-4 py-2 text-sm">
            <p>
              <code>{conflict.node}</code>{" "}
              <span className="text-muted-foreground">
                in {titleOf(conflict.doc_id)} ({String(conflict.kind).replace(/_/g, " ")})
              </span>
            </p>
            {conflict.resolution ? (
              <p>Resolved: {conflict.resolution.note}</p>
            ) : (
              <p className="text-amber-700 dark:text-amber-400">unresolved</p>
            )}
          </li>
        ))}
      </ul>
    </Section>
  );
}
