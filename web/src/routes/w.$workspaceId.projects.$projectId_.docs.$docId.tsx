import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import {
  createComment,
  holderOf,
  loadChangesets,
  loadDoc,
  loadDocComments,
  loadDraft,
  loadBoard,
  loadDraftHtml,
  loadVersionHtml,
  loadVersionOutline,
  loadVersions,
} from "@/api/docs";
import type { ChangesetBody, ClaimBody, CommentBody, DocBody, VersionBody, VersionItem } from "@/api/generated";
import {
  createDesignRenderer,
  type DesignRenderer,
  type Inspection,
  type Layer,
  type Property,
} from "@/design/renderer";
import { createAssetLoader } from "@/design/assets";
import { BoardCache, findPageHolding, pageWithBoards, showBoardwise } from "@/design/boards";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { useReloading, type Reloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";
import { ViewerFrame } from "@/viewer/frame";
import { blockIds, blockText } from "@/viewer/html";

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
// selected node, a switch between the design system's token modes, comment
// pins on the canvas, a version picker and the live draft of a change set.
//
// The `node` search parameter is a node address that a link from a post opens:
// the viewer selects it (a design shows the page that holds it first) and
// opens its comments.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/docs/$docId")({
  component: DocPage,
  validateSearch: (search: Record<string, unknown>): { node?: string } => ({
    node: typeof search.node === "string" && search.node ? search.node : undefined,
  }),
});

const TEXTAREA_CLASS =
  "min-h-20 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";
const SELECT_CLASS =
  "h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";

const DRAFT_PREFIX = "draft:";

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

/** The version picker's options, and what the picked option shows: a saved version or a live draft. */
function usePicked(
  doc: DocBody | null,
  versions: VersionItem[] | null,
  changesets: ChangesetBody[] | null,
  chosen: string | null,
) {
  const choices = useMemo(
    () => (doc && versions ? choicesOf(doc, versions, changesets) : []),
    [doc, versions, changesets],
  );
  const fallback = doc?.mainline_version_id ?? choices[0]?.value ?? null;
  const picked = chosen !== null && (choices.length === 0 || choices.some((c) => c.value === chosen)) ? chosen : fallback;
  const draft = picked?.startsWith(DRAFT_PREFIX) ? doc?.drafts.find((d) => DRAFT_PREFIX + d.id === picked) : null;
  const versionId = picked && !picked.startsWith(DRAFT_PREFIX) ? picked : null;
  return { choices, picked, draft, versionId };
}

function DocPage() {
  const { workspaceId, projectId, docId } = Route.useParams();
  const { node } = Route.useSearch();
  const doc = useReloading(useCallback(() => loadDoc(workspaceId, projectId, docId), [workspaceId, projectId, docId]));
  if (doc.value?.kind === "design") {
    return <DesignViewer workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc} node={node} />;
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
  return <HtmlDocViewer workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc} node={node} />;
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
  node,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
  node: string | undefined;
}) {
  const versions = useReloading(
    useCallback(() => loadVersions(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const changesets = useReloading(useCallback(() => loadChangesets(workspaceId, projectId), [workspaceId, projectId]));
  const comments = useReloading(
    useCallback(() => loadDocComments(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const [chosen, setChosen] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(node ?? null);
  const [versionHtml, setVersionHtml] = useState<Record<string, string>>({});

  // A link to another block of the doc selects that block.
  useEffect(() => {
    if (node) setSelected(node);
  }, [node]);

  const { choices, picked, draft, versionId } = usePicked(doc.value, versions.value, changesets.value, chosen);

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
              {draft && <ClaimsPanel claims={shownClaims} noun="block" />}
              <CommentsPanel
                workspaceId={workspaceId}
                projectId={projectId}
                selected={selected}
                comments={commentsOnSelected}
                versionId={versionId}
                noun="block"
                compare={(comment) => ({
                  then: blockTextOf(versionHtml[comment.version_id], selected),
                  now: blockTextOf(shownHtml, selected),
                })}
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

function ClaimsPanel({
  claims,
  noun,
  nameOf,
}: {
  claims: ClaimBody[];
  noun: string;
  /** The name of a claimed node; its ID when unknown. */
  nameOf?: (node: string) => string | undefined;
}) {
  return (
    <Panel id="claims" title="Claims">
      {claims.length === 0 && <p className="text-sm text-muted-foreground">No agent holds a claim.</p>}
      <ul className="divide-y rounded-md border empty:hidden">
        {claims.map((claim) => (
          <li key={claim.id} className="px-3 py-2 text-sm">
            <span className="font-medium">{holderOf(claim)}</span>
            <span className="text-muted-foreground">
              {" "}
              holds {noun} {nameOf?.(claim.node) ?? claim.node}
            </span>
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
  noun,
  compare,
  versionLabels,
  onPosted,
}: {
  workspaceId: string;
  projectId: string;
  selected: string | null;
  comments: CommentBody[];
  versionId: string | null;
  /** What a comment is on: a block of a doc, a node of a design. */
  noun: string;
  /** The selected node as it was on the version a comment was made on, and as it is on the version shown. */
  compare: (comment: CommentBody) => { then: string; now: string };
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
      {!selected && <p className="text-sm text-muted-foreground">Select a {noun} to see its comments.</p>}
      {selected && comments.length === 0 && (
        <p className="text-sm text-muted-foreground">No comments on this {noun} yet.</p>
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
                noun={noun}
                label={versionLabels[comment.version_id]}
                {...compare(comment)}
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

/** A comment made on another version than the one shown: the node as it was then, next to the node now. */
function Comparison({
  comment,
  noun,
  label,
  then,
  now,
}: {
  comment: CommentBody;
  noun: string;
  label: string | undefined;
  then: string;
  now: string;
}) {
  return (
    <dl className="grid gap-1 rounded-md bg-muted/50 p-2 text-xs">
      <dt className="font-medium">
        Made on {label ?? "an older version"}, the {noun} read
      </dt>
      <dd>{then}</dd>
      <dt className="font-medium">
        Now{comment.node_status === "removed" ? " (removed since)" : ""}
      </dt>
      <dd>{now}</dd>
    </dl>
  );
}

/** The text of a block of an HTML version, or why there is none. */
function blockTextOf(html: string | null | undefined, node: string | null): string {
  if (html === undefined || html === null) return "Loading";
  return (node ? blockText(html, node) : null) ?? "The block is not in this version.";
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

/**
 * What a design holds: its pages, the components its instances use and its
 * tokens. A saved version's pages are its outline, with every board a
 * placeholder, and its boards are in the version's `BoardCache`.
 */
type DesignSource = Pick<VersionBody, "pages" | "components" | "tokens">;

/** A version's outline as a source: its pages are the outline. */
function outlineSource(body: VersionBody): DesignSource {
  return { pages: body.outline ?? [], components: body.components, tokens: body.tokens };
}

/**
 * The node at `address` as the source holds it: a node ID, or
 * `<instanceId>/<componentNodeId>` for a node inside an instance, which is a
 * node of the instance's component.
 */
function nodeAt(source: DesignSource | null | undefined, address: string): JsonNode | null {
  if (!source) return null;
  const segments = address.split("/");
  if (segments.length === 1) return findNode(source.pages ?? [], address);
  if (!findNode(source.pages ?? [], segments[0])) return null;
  return findNode(source.components ?? [], segments[segments.length - 1]);
}

const NODE_ENVELOPE = new Set(["id", "type", "name", "children"]);

/** A node as the comparison writes it: a text's content, else its type, name and own fields. */
function describeNode(node: JsonNode | null): string {
  if (!node) return "The node is not in this version.";
  if (typeof node.content === "string") return node.content;
  const fields = Object.entries(node)
    .filter(([key]) => !NODE_ENVELOPE.has(key))
    .map(([key, value]) => `${key}: ${showValue(value)}`);
  const head = `${String(node.type)} ${String(node.name ?? "")}`.trim();
  return fields.length > 0 ? `${head}: ${fields.join(", ")}` : head;
}

/** Whether `page` holds a node with the ID `id`. */
function holds(page: JsonNode, id: string): boolean {
  return findNode([page], id) !== null;
}

/**
 * Loads the outline of each saved version asked for once; a saved version
 * never changes. A design version is read as its outline (its pages with their
 * boards as placeholders), never whole: its boards come one by one.
 */
function useVersionBodies(workspaceId: string, projectId: string, ids: string[]) {
  const [bodies, setBodies] = useState<Record<string, VersionBody>>({});
  const [failed, setFailed] = useState(false);
  const requested = useRef(new Set<string>());
  useEffect(() => {
    requested.current = new Set();
    setBodies({});
  }, [workspaceId, projectId]);
  useEffect(() => {
    for (const id of ids) {
      if (requested.current.has(id)) continue;
      requested.current.add(id);
      loadVersionOutline(workspaceId, projectId, id).then(
        (loaded) => {
          setBodies((previous) => ({ ...previous, [id]: loaded }));
          setFailed(false);
        },
        () => {
          requested.current.delete(id);
          setFailed(true);
        },
      );
    }
  }, [ids, workspaceId, projectId]);
  return { bodies, failed };
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
            className="pointer-events-auto absolute flex h-6 min-w-6 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-primary px-1.5 text-xs font-medium text-primary-foreground ring-2 ring-background aria-pressed:ring-ring"
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
 * The reviewer's view of a design doc: a version of it (the mainline version
 * at first) or a live draft on the renderer's canvas, a list of its pages and
 * a layers panel whose selection follows the canvas, and the other way
 * around. A version holds the pages, the components its instances use and the
 * tokens of the design system version it recorded, which the renderer
 * resolves the page against; a live draft holds the same, as the agents have
 * it now, and is loaded again on every operation applied to it.
 *
 * Comments pin on node addresses: a button over the canvas on each commented
 * node, and a Comments panel for the selected node, in which a new comment
 * attaches to the selected address and the version shown. A comment made on
 * another version shows the node as it was then next to the node now.
 */
function DesignViewer({
  workspaceId,
  projectId,
  docId,
  doc,
  node,
}: {
  workspaceId: string;
  projectId: string;
  docId: string;
  doc: Reloading<DocBody>;
  node: string | undefined;
}) {
  const versions = useReloading(
    useCallback(() => loadVersions(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const changesets = useReloading(useCallback(() => loadChangesets(workspaceId, projectId), [workspaceId, projectId]));
  const comments = useReloading(
    useCallback(() => loadDocComments(workspaceId, projectId, docId), [workspaceId, projectId, docId]),
  );
  const [chosenVersion, setChosenVersion] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [renderer, setRenderer] = useState<DesignRenderer | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [layers, setLayers] = useState<Layer[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  // The node a link opened, until it is selected.
  const [target, setTarget] = useState<string | null>(node ?? null);
  const [chosenMode, setChosenMode] = useState<string | null>(null);
  // Counts the times the renderer drew a page or another mode, so that the inspect panel asks again.
  const [drawn, setDrawn] = useState(0);
  const canvas = useRef<HTMLCanvasElement>(null);

  const { choices, draft, versionId } = usePicked(doc.value, versions.value, changesets.value, chosenVersion);
  const picked = draft ? DRAFT_PREFIX + draft.id : versionId;

  // The draft: loaded when it is picked and again on every operation applied to it.
  const draftChangeset = draft?.changeset_id;
  const loadLiveDraft = useCallback(
    () => (draftChangeset ? loadDraft(workspaceId, projectId, draftChangeset, docId) : Promise.resolve(null)),
    [workspaceId, projectId, draftChangeset, docId],
  );
  const liveDraft = useReloading(loadLiveDraft);

  // The version shown, and the versions that comments on the selected node were made on.
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
  const { bodies, failed: versionFailed } = useVersionBodies(workspaceId, projectId, wanted);
  const live = draft !== null && draft !== undefined;
  const shownBody = !live && versionId ? (bodies[versionId] ?? null) : null;
  const source: DesignSource | null = useMemo(
    () => (live ? liveDraft.value : shownBody ? outlineSource(shownBody) : null),
    [live, liveDraft.value, shownBody],
  );

  // The boards read of each saved version, so that a board is read once however
  // often its page is shown.
  const boardCaches = useRef(new Map<string, BoardCache>());
  useEffect(() => {
    boardCaches.current = new Map();
  }, [workspaceId, projectId, docId]);
  const cacheOf = useCallback(
    (version: string) => {
      let cache = boardCaches.current.get(version);
      if (!cache) {
        cache = new BoardCache((board) => loadBoard(workspaceId, projectId, docId, version, board));
        boardCaches.current.set(version, cache);
      }
      return cache;
    },
    [workspaceId, projectId, docId],
  );

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
    () => (source ? { ...source.tokens, $components: source.components ?? [] } : null),
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
      // The selection stays while its node is still there, as when a live draft changes.
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
        if (live || !versionId) {
          // A live draft is the agents' working copy, read whole.
          renderer.show(page, system);
          shown();
        } else {
          // A saved version shows its boards as placeholders, and reads those in view.
          stopBoards = showBoardwise({
            renderer,
            page,
            system,
            cache: cacheOf(versionId),
            prepare: async (board) => {
              problems.push(...(await assets.load(board, system)));
            },
            onShown: shown,
            onFailure: (message) => {
              problems.push(message);
              report();
            },
          });
        }
      } catch (error) {
        failed(error);
      }
    });
    return () => {
      cancelled = true;
      stopBoards?.();
    };
  }, [renderer, assets, page, system, modes, live, versionId, cacheOf]);

  // A link to a node: its page is shown, then it is selected.
  useEffect(() => {
    setTarget(node ?? null);
  }, [node]);
  useEffect(() => {
    if (!target) return;
    const holder = pages.find((candidate) => holds(candidate, target.split("/")[0]));
    if (holder && holder !== page) setChosen(String(holder.id));
  }, [target, pages, page]);
  // A node inside a board is on the page whose board holds it, which the boards say: they are read until one does.
  useEffect(() => {
    if (!target || live || !versionId || !assets || !system || pages.length === 0) return;
    const id = target.split("/")[0];
    if (pages.some((candidate) => holds(candidate, id))) return;
    let cancelled = false;
    void findPageHolding(
      cacheOf(versionId),
      pages,
      id,
      async (board) => {
        await assets.load(board, system);
      },
      () => cancelled,
    ).then((holder) => {
      if (!cancelled && holder && holder !== page) setChosen(String(holder.id));
    });
    return () => {
      cancelled = true;
    };
  }, [target, live, versionId, assets, system, pages, page, cacheOf]);
  useEffect(() => {
    if (!renderer || !target || !rowsOf(layers).some((row) => row.layer.address === target)) return;
    setSelected(target);
    renderer.select(target);
    setTarget(null);
  }, [renderer, target, layers]);

  useProjectEvents(projectId, (event) => {
    const { doc_id: eventDoc } = event.data;
    if (typeof eventDoc === "string" && eventDoc !== docId) return;
    const name = event.event;
    if (name === "op.applied") {
      if (draftChangeset) liveDraft.reload();
    } else if (name.startsWith("comment.")) {
      comments.reload();
    } else if (name.startsWith("claim.")) {
      doc.reload();
    } else {
      doc.reload();
      versions.reload();
      changesets.reload();
      if (draftChangeset) liveDraft.reload();
    }
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

  // The shown version with the boards read so far in place of their placeholders.
  const nowSource = useMemo(() => {
    if (!source || live || !versionId) return source;
    const cache = cacheOf(versionId);
    // `drawn` says that more boards were put on the canvas.
    return drawn < 0 ? source : { ...source, pages: (source.pages ?? []).map((entry) => pageWithBoards(entry, cache)) };
  }, [source, live, versionId, cacheOf, drawn]);

  // A comment made on another version compares the node with the same board of that version.
  const boardOfSelected = useMemo(() => {
    const id = selected?.split("/")[0];
    if (!id || !nowSource) return null;
    for (const entry of nowSource.pages ?? []) {
      const board = (Array.isArray(entry.children) ? (entry.children as JsonNode[]) : []).find((candidate) =>
        holds(candidate, id),
      );
      if (board) return String(board.id);
    }
    return null;
  }, [selected, nowSource]);
  const [olderBoards, setOlderBoards] = useState<Record<string, JsonNode | null>>({});
  const askedOlder = useRef(new Set<string>());
  useEffect(() => {
    askedOlder.current = new Set();
    setOlderBoards({});
  }, [workspaceId, projectId, docId]);
  useEffect(() => {
    if (!boardOfSelected) return;
    for (const comment of commentsOnSelected) {
      const key = `${comment.version_id}/${boardOfSelected}`;
      if (comment.version_id === versionId || askedOlder.current.has(key)) continue;
      askedOlder.current.add(key);
      loadBoard(workspaceId, projectId, docId, comment.version_id, boardOfSelected).then(
        (board) => setOlderBoards((previous) => ({ ...previous, [key]: board })),
        () => setOlderBoards((previous) => ({ ...previous, [key]: null })),
      );
    }
  }, [boardOfSelected, commentsOnSelected, versionId, workspaceId, projectId, docId]);
  function thenOf(version: string): string {
    const body = bodies[version];
    if (!body) return "Loading";
    if (!boardOfSelected) return "The earlier node cannot be shown.";
    const key = `${version}/${boardOfSelected}`;
    if (!(key in olderBoards)) return "Loading";
    const board = olderBoards[key];
    return describeNode(board ? nodeAt({ pages: [board], components: body.components }, selected ?? "") : null);
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

  // One pin on each node of the page shown that has comments, on any version.
  const pins = useMemo(() => {
    const names = new Map(rows.map(({ layer }) => [layer.address, layer.name]));
    const counts = new Map<string, number>();
    for (const comment of comments.value ?? []) {
      if (names.has(comment.node)) counts.set(comment.node, (counts.get(comment.node) ?? 0) + 1);
    }
    return Array.from(counts, ([address, count]) => ({ address, count, name: names.get(address) ?? address }));
  }, [rows, comments.value]);

  const shownClaims = useMemo(
    () => (draft ? (doc.value?.claims ?? []).filter((claim) => claim.draft_id === draft.id) : []),
    [draft, doc.value],
  );

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
          <div className="flex flex-wrap items-end gap-4">
            <div className="w-80 max-w-full space-y-1">
              <Label htmlFor="version-picker">Version</Label>
              <select
                id="version-picker"
                className={SELECT_CLASS}
                value={picked ?? ""}
                onChange={(event) => setChosenVersion(event.target.value)}
              >
                {choices.map((choice) => (
                  <option key={choice.value} value={choice.value}>
                    {choice.label}
                  </option>
                ))}
              </select>
            </div>
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
          </div>
        </header>
      )}
      {(versionFailed || liveDraft.failed) && (
        <Alert variant="destructive">
          <AlertDescription>The design could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {failure && (
        <Alert variant="destructive">
          <AlertDescription>{failure}</AlertDescription>
        </Alert>
      )}
      {doc.value && !picked && <p className="text-sm text-muted-foreground">This doc has no saved version yet.</p>}
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
          <CommentPins renderer={renderer} pins={pins} selected={selected} onPick={choose} />
        </div>
        <div className="space-y-6">
          {draft && (
            <ClaimsPanel
              claims={shownClaims}
              noun="board"
              nameOf={(claimed) => {
                const found = findNode(pages, claimed);
                return typeof found?.name === "string" ? found.name : undefined;
              }}
            />
          )}
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
          <CommentsPanel
            workspaceId={workspaceId}
            projectId={projectId}
            selected={selected}
            comments={commentsOnSelected}
            versionId={versionId}
            noun="node"
            compare={(comment) => ({
              then: thenOf(comment.version_id),
              now: nowSource ? describeNode(nodeAt(nowSource, selected ?? "")) : "Loading",
            })}
            versionLabels={Object.fromEntries(choices.map((choice) => [choice.value, choice.label]))}
            onPosted={comments.reload}
          />
          <InspectPanel inspection={inspection} />
        </div>
      </div>
    </main>
  );
}
