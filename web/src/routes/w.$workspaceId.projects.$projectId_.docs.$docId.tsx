import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import {
  createComment,
  holderOf,
  loadChangesets,
  loadDoc,
  loadDocComments,
  loadDraftHtml,
  loadVersion,
  loadVersionHtml,
  loadVersions,
} from "@/api/docs";
import type { ChangesetBody, ClaimBody, CommentBody, DocBody, VersionItem } from "@/api/generated";
import {
  createDesignRenderer,
  type DesignRenderer,
  type Inspection,
  type Layer,
  type Property,
} from "@/design/renderer";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { useProjectEvents } from "@/live/context";
import { viewerDocument } from "@/viewer/document";
import {
  attachmentIds,
  blockIds,
  blockText,
  loadAttachmentImage,
  withAttachmentImages,
} from "@/viewer/html";

// The doc viewer is where the reviewer reads an HTML doc and comments on its
// blocks. The doc is shown in a sandboxed frame (src/viewer/document.ts) that
// the page talks to by postMessage only: the frame tells which block was
// clicked, and the page tells it what to show, which block is selected, where
// the pins are and which claims are held. A version picker chooses the version
// shown (the mainline version first) or the live draft of a change set, which
// is loaded again on every operation an agent applies. Comments attach to the
// selected block and the version shown.
//
// A design doc is shown by the design viewer instead (further down): the
// renderer's canvas with a page list, a layers panel, an inspect panel for the
// selected node and a switch between the design system's token modes.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/$docId")({
  component: DocPage,
});

const TEXTAREA_CLASS =
  "min-h-20 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";
const SELECT_CLASS =
  "h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";

const DRAFT_PREFIX = "draft:";

/** Loads a value, and again after each call of `reload`; an answer that is no longer the newest is dropped. */
function useReloading<T>(load: () => Promise<T>) {
  const [value, setValue] = useState<T | null>(null);
  const [failed, setFailed] = useState(false);
  const newest = useRef(0);

  const reload = useCallback(() => {
    newest.current += 1;
    const mine = newest.current;
    load().then(
      (loaded) => {
        if (mine !== newest.current) return;
        setValue(loaded);
        setFailed(false);
      },
      () => {
        if (mine === newest.current) setFailed(true);
      },
    );
  }, [load]);

  useEffect(() => {
    setValue(null);
    reload();
    return () => {
      newest.current += 1;
    };
  }, [reload]);
  return { value, failed, reload };
}

type Reloading<T> = ReturnType<typeof useReloading<T>>;

function holderOfComment(author: Record<string, unknown>): string {
  return author.kind === "human" ? "Reviewer" : `${String(author.run)}/${String(author.role)}`;
}

function stamp(createdAt: string): string {
  return createdAt.slice(0, 16).replace("T", " ");
}

type Choice = { value: string; label: string };

/** The picker's options: the mainline version first, the other versions newest first, then the drafts. */
function choicesOf(doc: DocBody, versions: VersionItem[], changesets: ChangesetBody[] | null): Choice[] {
  const titleOf = (id: string) => changesets?.find((changeset) => changeset.id === id)?.title ?? id.slice(0, 8);
  const label = (version: VersionItem) =>
    [
      titleOf(version.changeset_id),
      stamp(version.created_at),
      holderOfComment(version.agent),
      version.id === doc.mainline_version_id ? "mainline" : null,
      version.removed ? "removed" : null,
    ]
      .filter(Boolean)
      .join(" · ");
  const newestFirst = [...versions].reverse();
  const ordered = [
    ...newestFirst.filter((version) => version.id === doc.mainline_version_id),
    ...newestFirst.filter((version) => version.id !== doc.mainline_version_id),
  ];
  const live = (changesetId: string) => {
    const status = changesets?.find((changeset) => changeset.id === changesetId)?.status;
    return status !== "merged" && status !== "abandoned";
  };
  const drafts = doc.drafts
    .filter((draft) => live(draft.changeset_id))
    .map((draft) => ({
      value: `${DRAFT_PREFIX}${draft.id}`,
      label: `Live draft · ${titleOf(draft.changeset_id)}`,
    }));
  return [...ordered.map((version) => ({ value: version.id, label: label(version) })), ...drafts];
}

function DocPage() {
  const { workspaceId, projectId, docId } = Route.useParams();
  const doc = useReloading(useCallback(() => loadDoc(workspaceId, projectId, docId), [workspaceId, projectId, docId]));
  if (doc.value?.kind === "design") {
    return <DesignViewer workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc} />;
  }
  if (!doc.value) {
    return (
      <main className="mx-auto max-w-7xl space-y-6 p-6">
        <BackLink workspaceId={workspaceId} projectId={projectId} />
        {doc.failed && (
          <Alert variant="destructive">
            <AlertDescription>The doc could not be loaded. Try again in a moment.</AlertDescription>
          </Alert>
        )}
      </main>
    );
  }
  return <HtmlDocViewer workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc} />;
}

function BackLink({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  return (
    <Link
      to="/w/$workspaceId/projects/$projectId"
      params={{ workspaceId, projectId }}
      className="text-sm text-muted-foreground underline-offset-4 hover:underline"
    >
      Back to the project
    </Link>
  );
}

function HtmlDocViewer({
  workspaceId,
  projectId,
  docId,
  doc,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
}) {
  const versions = useReloading(
    useCallback(() => loadVersions(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const changesets = useReloading(useCallback(() => loadChangesets(workspaceId, projectId), [workspaceId, projectId]));
  const comments = useReloading(
    useCallback(() => loadDocComments(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const [chosen, setChosen] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [versionHtml, setVersionHtml] = useState<Record<string, string>>({});

  const choices = useMemo(
    () => (doc.value && versions.value ? choicesOf(doc.value, versions.value, changesets.value) : []),
    [doc.value, versions.value, changesets.value],
  );
  const fallback = doc.value?.mainline_version_id ?? choices[0]?.value ?? null;
  const picked = chosen !== null && (choices.length === 0 || choices.some((c) => c.value === chosen)) ? chosen : fallback;
  const draft = picked?.startsWith(DRAFT_PREFIX) ? doc.value?.drafts.find((d) => DRAFT_PREFIX + d.id === picked) : null;
  const versionId = picked && !picked.startsWith(DRAFT_PREFIX) ? picked : null;

  // The draft: loaded when it is picked and again on every operation applied to it.
  const draftChangeset = draft?.changeset_id;
  const loadDraft = useCallback(
    () => (draftChangeset ? loadDraftHtml(workspaceId, projectId, draftChangeset, docId) : Promise.resolve(null)),
    [workspaceId, projectId, draftChangeset, docId],
  );
  const draftSource = useReloading(loadDraft);
  const draftHtml = draftSource.value;

  // Saved versions never change, so each is loaded once: the version shown and
  // the versions that comments on the selected block were made on.
  const commentsOnSelected = useMemo(
    () => (comments.value ?? []).filter((comment) => comment.node === selected),
    [comments.value, selected],
  );
  const wanted = useMemo(() => {
    const ids = new Set<string>();
    if (versionId) ids.add(versionId);
    for (const comment of commentsOnSelected) ids.add(comment.version_id);
    return Array.from(ids);
  }, [versionId, commentsOnSelected]);
  const requested = useRef(new Set<string>());
  useEffect(() => {
    requested.current = new Set();
    setVersionHtml({});
  }, [workspaceId, projectId, docId]);
  useEffect(() => {
    for (const id of wanted) {
      if (requested.current.has(id)) continue;
      requested.current.add(id);
      loadVersionHtml(workspaceId, projectId, docId, id).then(
        (loaded) => setVersionHtml((previous) => ({ ...previous, [id]: loaded })),
        () => requested.current.delete(id),
      );
    }
  }, [wanted, workspaceId, projectId, docId]);

  useProjectEvents(projectId, (event) => {
    const { doc_id: eventDoc } = event.data;
    if (typeof eventDoc === "string" && eventDoc !== docId) return;
    const name = event.event;
    if (name === "op.applied") {
      if (draftChangeset) draftSource.reload();
    } else if (name.startsWith("comment.")) {
      comments.reload();
    } else if (name.startsWith("claim.")) {
      doc.reload();
    } else {
      doc.reload();
      versions.reload();
      changesets.reload();
      if (draftChangeset) draftSource.reload();
    }
  });

  const shownHtml = draft ? draftHtml : versionId ? (versionHtml[versionId] ?? null) : null;
  const shownClaims = useMemo(
    () => (draft ? (doc.value?.claims ?? []).filter((claim) => claim.draft_id === draft.id) : []),
    [draft, doc.value],
  );
  const pins = useMemo(() => {
    if (shownHtml === null) return [];
    const present = blockIds(shownHtml);
    const counts = new Map<string, number>();
    for (const comment of comments.value ?? []) {
      if (present.has(comment.node)) counts.set(comment.node, (counts.get(comment.node) ?? 0) + 1);
    }
    return Array.from(counts, ([node, count]) => ({ node, count }));
  }, [shownHtml, comments.value]);

  return (
    <main className="mx-auto max-w-7xl space-y-6 p-6">
      <BackLink workspaceId={workspaceId} projectId={projectId} />
      {doc.failed && (
        <Alert variant="destructive">
          <AlertDescription>The doc could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {doc.value && (
        <>
          <header className="flex flex-wrap items-end justify-between gap-4">
            <h1 className="text-2xl font-semibold">{doc.value.title}</h1>
            <div className="w-full max-w-md space-y-1">
              <Label htmlFor="version-picker">Version</Label>
              <select
                id="version-picker"
                className={SELECT_CLASS}
                value={picked ?? ""}
                onChange={(event) => setChosen(event.target.value)}
              >
                {choices.map((choice) => (
                  <option key={choice.value} value={choice.value}>
                    {choice.label}
                  </option>
                ))}
              </select>
            </div>
          </header>
          {choices.length === 0 && versions.value && (
            <p className="text-sm text-muted-foreground">This doc has no saved version yet.</p>
          )}
          <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_22rem]">
            <ViewerFrame
              workspaceId={workspaceId}
              html={shownHtml}
              pins={pins}
              selected={selected}
              claims={shownClaims}
              onSelect={setSelected}
            />
            <aside className="space-y-6">
              {draft && <ClaimsPanel claims={shownClaims} />}
              <CommentsPanel
                workspaceId={workspaceId}
                projectId={projectId}
                selected={selected}
                comments={commentsOnSelected}
                versionId={versionId}
                shownHtml={shownHtml}
                versionHtml={versionHtml}
                versionLabels={Object.fromEntries(choices.map((choice) => [choice.value, choice.label]))}
                onPosted={comments.reload}
              />
            </aside>
          </div>
        </>
      )}
    </main>
  );
}

/** The sandboxed frame that shows the doc, and the messages between it and the page. */
function ViewerFrame({
  workspaceId,
  html,
  pins,
  selected,
  claims,
  onSelect,
}: {
  workspaceId: string;
  html: string | null;
  pins: { node: string; count: number }[];
  selected: string | null;
  claims: ClaimBody[];
  onSelect: (node: string) => void;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [ready, setReady] = useState(0);
  const srcDoc = useMemo(() => viewerDocument({ workspaceId, origin: window.location.origin }), [workspaceId]);
  const onSelectRef = useRef(onSelect);
  useEffect(() => {
    onSelectRef.current = onSelect;
  });

  // Images of attachments are read with the reviewer's session and given to
  // the frame as data: URLs, because the frame's own requests carry no session.
  const [images, setImages] = useState<Record<string, string>>({});
  const requested = useRef(new Set<string>());
  useEffect(() => {
    for (const id of html ? attachmentIds(html) : []) {
      if (requested.current.has(id)) continue;
      requested.current.add(id);
      loadAttachmentImage(workspaceId, id).then(
        (url) => setImages((previous) => ({ ...previous, [id]: url })),
        () => requested.current.delete(id),
      );
    }
  }, [html, workspaceId]);
  const shown = useMemo(() => (html === null ? null : withAttachmentImages(html, images)), [html, images]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (!frame.current || event.source !== frame.current.contentWindow) return;
      const message = event.data as { type?: unknown; node?: unknown } | null;
      if (message?.type === "viewer.ready") setReady((count) => count + 1);
      else if (message?.type === "viewer.select" && typeof message.node === "string") {
        onSelectRef.current(message.node);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  useEffect(() => {
    if (shown === null || !ready) return;
    // The frame's origin is opaque, so "*" is the only target a page can name.
    frame.current?.contentWindow?.postMessage({ type: "viewer.show", html: shown, pins, selected, claims }, "*");
  }, [shown, pins, selected, claims, ready]);

  return (
    <iframe
      ref={frame}
      title="Doc"
      sandbox="allow-scripts"
      srcDoc={srcDoc}
      className="h-[75vh] w-full rounded-md border bg-white"
    />
  );
}

function Panel({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  return (
    <section aria-labelledby={id} className="space-y-3">
      <h2 id={id} className="text-lg font-semibold">
        {title}
      </h2>
      {children}
    </section>
  );
}

function ClaimsPanel({ claims }: { claims: ClaimBody[] }) {
  return (
    <Panel id="claims" title="Claims">
      {claims.length === 0 && <p className="text-sm text-muted-foreground">No agent holds a claim.</p>}
      <ul className="divide-y rounded-md border empty:hidden">
        {claims.map((claim) => (
          <li key={claim.id} className="px-3 py-2 text-sm">
            <span className="font-medium">{holderOf(claim)}</span>
            <span className="text-muted-foreground"> holds block {claim.node}</span>
          </li>
        ))}
      </ul>
    </Panel>
  );
}

function CommentsPanel({
  workspaceId,
  projectId,
  selected,
  comments,
  versionId,
  shownHtml,
  versionHtml,
  versionLabels,
  onPosted,
}: {
  workspaceId: string;
  projectId: string;
  selected: string | null;
  comments: CommentBody[];
  versionId: string | null;
  shownHtml: string | null;
  versionHtml: Record<string, string>;
  versionLabels: Record<string, string>;
  onPosted: () => void;
}) {
  const [text, setText] = useState("");
  const [posting, setPosting] = useState(false);
  const [failed, setFailed] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const said = text.trim();
    if (!selected || !versionId || !said || posting) return;
    setPosting(true);
    setFailed(false);
    try {
      await createComment(workspaceId, projectId, versionId, { node: selected, text: said });
      setText("");
      onPosted();
    } catch {
      setFailed(true);
    } finally {
      setPosting(false);
    }
  }

  return (
    <Panel id="comments" title="Comments">
      {!selected && <p className="text-sm text-muted-foreground">Select a block of the doc to see its comments.</p>}
      {selected && comments.length === 0 && (
        <p className="text-sm text-muted-foreground">No comments on this block yet.</p>
      )}
      <ul className="space-y-3 empty:hidden">
        {comments.map((comment) => (
          <li key={comment.id} className="space-y-2 rounded-md border p-3 text-sm">
            <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
              <span>{holderOfComment(comment.author)}</span>
              <span className="rounded-full border px-2 py-0.5">{comment.status}</span>
            </div>
            <p>{comment.text}</p>
            {comment.version_id !== versionId && selected && (
              <Comparison
                comment={comment}
                node={selected}
                label={versionLabels[comment.version_id]}
                then={versionHtml[comment.version_id]}
                now={shownHtml}
              />
            )}
            {comment.replies.map((reply) => (
              <p key={reply.id} className="border-l-2 pl-2 text-muted-foreground">
                {reply.text}
              </p>
            ))}
          </li>
        ))}
      </ul>
      {selected && versionId && (
        <form onSubmit={submit} className="space-y-2">
          <Label htmlFor="new-comment">Comment</Label>
          <textarea
            id="new-comment"
            className={TEXTAREA_CLASS}
            value={text}
            onChange={(event) => setText(event.target.value)}
          />
          {failed && (
            <Alert variant="destructive">
              <AlertDescription>The comment could not be saved.</AlertDescription>
            </Alert>
          )}
          <Button type="submit" disabled={posting || !text.trim()}>
            Comment
          </Button>
        </form>
      )}
      {selected && !versionId && (
        <p className="text-sm text-muted-foreground">A live draft cannot be commented on. Pick a saved version.</p>
      )}
    </Panel>
  );
}

/** A comment made on another version than the one shown: the block as it was then, next to the block now. */
function Comparison({
  comment,
  node,
  label,
  then,
  now,
}: {
  comment: CommentBody;
  node: string;
  label: string | undefined;
  then: string | undefined;
  now: string | null;
}) {
  const textOf = (html: string | null | undefined) =>
    html === undefined || html === null ? "Loading" : (blockText(html, node) ?? "The block is not in this version.");
  return (
    <dl className="grid gap-1 rounded-md bg-muted/50 p-2 text-xs">
      <dt className="font-medium">Made on {label ?? "an older version"}, the block read</dt>
      <dd>{textOf(then)}</dd>
      <dt className="font-medium">
        Now{comment.node_status === "removed" ? " (removed since)" : ""}
      </dt>
      <dd>{textOf(now)}</dd>
    </dl>
  );
}

// The design viewer ------------------------------------------------------------------------

/** A layer and its depth, in the order the tree lists them. */
type Row = { layer: Layer; level: number };

function rowsOf(layers: Layer[], level = 1): Row[] {
  return layers.flatMap((layer) => [{ layer, level }, ...rowsOf(layer.children, level + 1)]);
}

/** The modes of a design system version's token tree, in order. */
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

function InspectTable({
  label,
  rows,
}: {
  label: string;
  rows: { name: string; cells: ReactNode }[];
}) {
  return (
    <table aria-label={label} className="w-full text-left text-xs">
      <caption className="pb-1 text-left text-sm font-medium">{label}</caption>
      <tbody className="divide-y">
        {rows.map((row) => (
          <tr key={row.name} className="align-top">
            <th scope="row" className="py-1 pr-2 font-medium whitespace-nowrap">
              {row.name}
            </th>
            <td className="py-1 break-all">{row.cells}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function PropertyCells({ property, laidOut }: { property: Property; laidOut?: number }) {
  return (
    <>
      <span className="font-mono">{showValue(property.value)}</span>
      {laidOut !== undefined && <span className="text-muted-foreground"> ({NUMBER_FORMAT.format(laidOut)} px)</span>}
      {property.tokens.map((used) => (
        <span key={used.token} className="block text-muted-foreground">
          <span className="font-mono">{used.token}</span> <span className="font-mono">{showValue(used.value)}</span>
        </span>
      ))}
    </>
  );
}

/** Every property of the selected node, with the tokens it uses and their values in the current mode. */
function InspectPanel({ inspection }: { inspection: Inspection | null }) {
  const group = (label: string, properties: Property[], laidOut?: (name: string) => number | undefined) =>
    properties.length > 0 && (
      <InspectTable
        label={label}
        rows={properties.map((property) => ({
          name: property.name,
          cells: <PropertyCells property={property} laidOut={laidOut?.(property.name)} />,
        }))}
      />
    );
  const instance = inspection?.instance;
  return (
    <Panel id="inspect" title="Inspect">
      {!inspection && <p className="text-sm text-muted-foreground">Select a layer to inspect it.</p>}
      {inspection && (
        <div className="space-y-4">
          <p className="text-sm font-medium">
            {inspection.name} <span className="text-xs font-normal text-muted-foreground">{inspection.type}</span>
          </p>
          {group("Layout", inspection.layout)}
          {group("Size", inspection.size, (name) =>
            name === "width" ? inspection.box.width : name === "height" ? inspection.box.height : undefined,
          )}
          {group("Style", inspection.style)}
          {group("Text", inspection.text)}
          {instance && (
            <InspectTable
              label="Component"
              rows={[
                { name: "component", cells: <span className="font-mono">{instance.componentName}</span> },
                ...instance.props.map((prop) => ({
                  name: prop.name,
                  cells: (
                    <>
                      <span className="font-mono">{showValue(prop.value)}</span>
                      <span className="text-muted-foreground">
                        {" "}
                        {prop.kind}
                        {prop.set ? "" : ", default"}
                      </span>
                    </>
                  ),
                })),
              ]}
            />
          )}
          {instance && instance.overrides.length > 0 && (
            <InspectTable
              label="Overrides"
              rows={instance.overrides.map((override) => ({
                name: override.name,
                cells: <span className="font-mono">{showValue(override.fields)}</span>,
              }))}
            />
          )}
        </div>
      )}
    </Panel>
  );
}

/**
 * The reviewer's view of a design doc: its mainline version on the renderer's
 * canvas, a list of its pages and a layers panel whose selection follows the
 * canvas, and the other way around. The version holds the pages, the
 * components its instances use and the tokens of the design system version it
 * recorded, which the renderer resolves the page against.
 */
function DesignViewer({
  workspaceId,
  projectId,
  docId,
  doc,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
}) {
  const mainline = doc.value?.mainline_version_id ?? null;
  // A saved version never changes, so it is loaded again only for another one.
  const version = useReloading(
    useCallback(
      () => (mainline ? loadVersion(workspaceId, projectId, mainline) : Promise.resolve(null)),
      [workspaceId, projectId, mainline],
    ),
  );
  const [chosen, setChosen] = useState<string | null>(null);
  const [renderer, setRenderer] = useState<DesignRenderer | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [layers, setLayers] = useState<Layer[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [chosenMode, setChosenMode] = useState<string | null>(null);
  // Counts the times the renderer drew a page or another mode, so that the inspect panel asks again.
  const [drawn, setDrawn] = useState(0);
  const canvas = useRef<HTMLCanvasElement>(null);

  const modes = useMemo(() => modesOf(version.value?.tokens), [version.value]);
  const mode = chosenMode !== null && modes.includes(chosenMode) ? chosenMode : (modes[0] ?? null);
  const modeRef = useRef(mode);
  useEffect(() => {
    modeRef.current = mode;
  });

  const pages = useMemo(() => version.value?.pages ?? [], [version.value]);
  const page = useMemo(() => pages.find((candidate) => candidate.id === chosen) ?? pages[0] ?? null, [pages, chosen]);
  const system = useMemo(
    () => (version.value ? { ...version.value.tokens, $components: version.value.components ?? [] } : null),
    [version.value],
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

  useEffect(() => {
    if (!renderer || !page || !system) return;
    try {
      renderer.show(page, system);
      // A page is shown in the first mode, so the chosen mode is set again.
      if (modeRef.current && modeRef.current !== modes[0]) renderer.setMode(modeRef.current);
      setDrawn((count) => count + 1);
      setLayers(renderer.layers());
      setSelected(null);
      setFailure(null);
    } catch (error) {
      setFailure(error instanceof Error ? error.message : "The design could not be drawn.");
    }
  }, [renderer, page, system, modes]);

  useProjectEvents(projectId, (event) => {
    const { doc_id: eventDoc } = event.data;
    if (typeof eventDoc === "string" && eventDoc !== docId) return;
    if (!event.event.startsWith("comment.")) doc.reload();
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

  return (
    <main className="mx-auto max-w-7xl space-y-6 p-6">
      <BackLink workspaceId={workspaceId} projectId={projectId} />
      {doc.failed && (
        <Alert variant="destructive">
          <AlertDescription>The doc could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {doc.value && (
        <header className="flex flex-wrap items-end justify-between gap-4">
          <h1 className="text-2xl font-semibold">{doc.value.title}</h1>
          {mode !== null && (
            <div className="w-40 space-y-1">
              <Label htmlFor="mode-switch">Mode</Label>
              <select
                id="mode-switch"
                className={SELECT_CLASS}
                value={mode}
                onChange={(event) => chooseMode(event.target.value)}
              >
                {modes.map((listed) => (
                  <option key={listed} value={listed}>
                    {listed}
                  </option>
                ))}
              </select>
            </div>
          )}
        </header>
      )}
      {version.failed && (
        <Alert variant="destructive">
          <AlertDescription>The design could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {failure && (
        <Alert variant="destructive">
          <AlertDescription>{failure}</AlertDescription>
        </Alert>
      )}
      {doc.value && !mainline && <p className="text-sm text-muted-foreground">This doc has no saved version yet.</p>}
      <div className="grid gap-6 lg:grid-cols-[14rem_minmax(0,1fr)_18rem]">
        <nav aria-label="Pages" className="space-y-2">
          <h2 className="text-lg font-semibold">Pages</h2>
          <ul className="space-y-1">
            {pages.map((candidate) => (
              <li key={String(candidate.id)}>
                <Button
                  type="button"
                  variant={candidate === page ? "secondary" : "ghost"}
                  className="w-full justify-start"
                  aria-current={candidate === page ? "page" : undefined}
                  onClick={() => setChosen(String(candidate.id))}
                >
                  {String(candidate.name ?? candidate.id)}
                </Button>
              </li>
            ))}
          </ul>
        </nav>
        <div className="relative h-[75vh] overflow-hidden rounded-md border bg-muted/30">
          <canvas ref={canvas} aria-label="Design canvas" className="size-full touch-none" />
        </div>
        <div className="space-y-6">
          <Panel id="layers" title="Layers">
            <div role="tree" aria-label="Layers of the page" className="max-h-[70vh] overflow-auto text-sm">
              {rows.map(({ layer, level }) => (
                <div
                  key={layer.address}
                  role="treeitem"
                  tabIndex={0}
                  aria-level={level}
                  aria-selected={layer.address === selected}
                  style={{ paddingLeft: `${level * 0.75}rem` }}
                  className="cursor-pointer rounded-sm py-1 pr-2 outline-none hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 aria-selected:bg-accent aria-selected:font-medium"
                  onClick={() => choose(layer.address)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      choose(layer.address);
                    }
                  }}
                >
                  {layer.name} <span className="text-xs text-muted-foreground">{layer.type}</span>
                </div>
              ))}
            </div>
          </Panel>
          <InspectPanel inspection={inspection} />
        </div>
      </div>
    </main>
  );
}
