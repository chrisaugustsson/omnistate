import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { createFileRoute } from "@tanstack/react-router";
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
import {
  FileText,
  Files,
  Layers as LayersIcon,
  Lock,
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
  LiveIndicator,
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
  "block min-h-20 w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-[13px] leading-relaxed outline-none placeholder:text-muted-foreground";

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
      <main className="flex min-h-full flex-col">
        <ViewerToolbar workspaceId={workspaceId} projectId={projectId} title={null} />
        <div className="relative flex-1 bg-canvas">
          <FloatingAlerts>
            {doc.failed && <FloatingAlert>The doc could not be loaded. Try again in a moment.</FloatingAlert>}
          </FloatingAlerts>
        </div>
      </main>
    );
  }
  return <HtmlDocViewer workspaceId={workspaceId} projectId={projectId} docId={docId} doc={doc} node={node} />;
}

/** The version picker: the versions, the mainline first, and the live drafts. */
function VersionPicker({
  choices,
  picked,
  onPick,
}: {
  choices: Choice[];
  picked: string | null;
  onPick: (value: string) => void;
}) {
  return (
    <ToolbarSelect
      id="version-picker"
      label="Version"
      className="w-full sm:w-72"
      value={picked ?? ""}
      onChange={(event) => onPick(event.target.value)}
    >
      {choices.map((choice) => (
        <option key={choice.value} value={choice.value}>
          {choice.label}
        </option>
      ))}
    </ToolbarSelect>
  );
}

/** The holders of the claims shown, as the live indicator names them. */
function holdersOf(claims: ClaimBody[]): string[] {
  return claims.map((claim) => holderOf(claim));
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
    for (const comment of commentsOnSelected) if (comment.version_id) ids.add(comment.version_id);
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
    <main className="flex min-h-full flex-col lg:h-full lg:min-h-[560px]">
      <ViewerToolbar
        workspaceId={workspaceId}
        projectId={projectId}
        featureId={doc.value?.feature_id}
        icon={<FileText />}
        title={doc.value?.title ?? null}
        status={draft && <LiveIndicator holders={holdersOf(shownClaims)} />}
      >
        {doc.value && <VersionPicker choices={choices} picked={picked} onPick={setChosen} />}
      </ViewerToolbar>
      <div className="flex min-h-0 flex-1 flex-col lg:grid lg:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="relative flex min-h-0 flex-col bg-canvas px-3 py-4 sm:px-6 sm:py-6 lg:px-8">
          <FloatingAlerts>
            {doc.failed && <FloatingAlert>The doc could not be loaded. Try again in a moment.</FloatingAlert>}
          </FloatingAlerts>
          {doc.value && choices.length === 0 && versions.value && (
            <div className="mx-auto mb-4 w-full max-w-[56rem]">
              <PaneEmpty icon={<FileText />}>This doc has no saved version yet.</PaneEmpty>
            </div>
          )}
          {doc.value && (
            <div className="mx-auto flex h-[78vh] min-h-0 w-full max-w-[56rem] flex-col overflow-hidden rounded-xl bg-white shadow-panel ring-1 ring-black/[0.04] lg:h-auto lg:flex-1">
              <ViewerFrame
                workspaceId={workspaceId}
                html={shownHtml}
                pins={pins}
                selected={selected}
                claims={shownClaims}
                onSelect={setSelected}
                className="block min-h-0 w-full flex-1 bg-white"
              />
            </div>
          )}
        </div>
        {doc.value && (
          <aside
            aria-label="Review"
            className="border-t bg-background lg:min-h-0 lg:overflow-y-auto lg:border-t-0 lg:border-l"
          >
            {draft && <ClaimsPanel claims={shownClaims} noun="block" />}
            <CommentsPanel
              workspaceId={workspaceId}
              projectId={projectId}
              selected={selected}
              comments={commentsOnSelected}
              versionId={versionId}
              noun="block"
              compare={(comment) => ({
                then: blockTextOf(versionHtml[comment.version_id ?? ""], selected),
                now: blockTextOf(shownHtml, selected),
              })}
              versionLabels={Object.fromEntries(choices.map((choice) => [choice.value, choice.label]))}
              onPosted={comments.reload}
            />
          </aside>
        )}
      </div>
    </main>
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
    <PaneSection id="claims" title="Claims" aside={claims.length > 0 ? claims.length : undefined}>
      {claims.length === 0 && <PaneEmpty icon={<Lock />}>No agent holds a claim.</PaneEmpty>}
      <ul className="space-y-1 empty:hidden">
        {claims.map((claim) => (
          <li
            key={claim.id}
            className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 rounded-lg bg-signal-soft/60 px-2.5 py-2 text-[12.5px] ring-1 ring-signal/15 ring-inset"
          >
            <Lock aria-hidden className="size-3 shrink-0 text-signal" />
            <AgentChip name={holderOf(claim)} />
            <span className="text-muted-foreground">
              {" "}
              holds {noun} <span className="font-medium text-foreground">{nameOf?.(claim.node) ?? claim.node}</span>
            </span>
          </li>
        ))}
      </ul>
    </PaneSection>
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
            {comment.version_id && comment.version_id !== versionId && selected && (
              <Comparison
                noun={noun}
                label={versionLabels[comment.version_id]}
                {...compare(comment)}
              />
            )}
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
      {selected && versionId && (
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
      {selected && !versionId && (
        <p className="mt-3 flex items-start gap-2 rounded-lg bg-signal-soft/60 px-3 py-2 text-[12px] leading-snug text-muted-foreground">
          A live draft cannot be commented on. Pick a saved version.
        </p>
      )}
    </PaneSection>
  );
}

/** A comment made on another version than the one shown: the node as it was then, next to the node now. */
function Comparison({
  noun,
  label,
  then,
  now,
}: {
  noun: string;
  label: string | undefined;
  then: string;
  now: string;
}) {
  return (
    <dl className="grid gap-1 rounded-md bg-muted p-2 text-[12px] leading-snug">
      <dt className="text-[11px] font-medium text-muted-foreground">
        Made on {label ?? "an older version"}, the {noun} read
      </dt>
      <dd className="break-words text-muted-foreground">{then}</dd>
      <dt className="mt-1 text-[11px] font-medium text-muted-foreground">
        Now
      </dt>
      <dd className="break-words">{now}</dd>
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
    for (const comment of commentsOnSelected) if (comment.version_id) ids.add(comment.version_id);
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
      const older = comment.version_id;
      if (!older || older === versionId) continue;
      const key = `${older}/${boardOfSelected}`;
      if (askedOlder.current.has(key)) continue;
      askedOlder.current.add(key);
      loadBoard(workspaceId, projectId, docId, older, boardOfSelected).then(
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
    <main className="flex min-h-full flex-col lg:h-full lg:min-h-[560px]">
      <ViewerToolbar
        workspaceId={workspaceId}
        projectId={projectId}
        featureId={doc.value?.feature_id}
        icon={<PenTool />}
        title={doc.value?.title ?? null}
        status={draft && <LiveIndicator holders={holdersOf(shownClaims)} />}
      >
        {doc.value && (
          <>
            <VersionPicker choices={choices} picked={picked} onPick={setChosenVersion} />
            {mode !== null && (
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
          </>
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
          {doc.value && !picked && (
            <div className="pointer-events-none absolute inset-0 flex items-center justify-center p-6">
              <p className="rounded-xl bg-background/90 px-4 py-3 text-[13px] text-muted-foreground shadow-panel">
                This doc has no saved version yet.
              </p>
            </div>
          )}
          <FloatingAlerts>
            {doc.failed && <FloatingAlert>The doc could not be loaded. Try again in a moment.</FloatingAlert>}
            {(versionFailed || liveDraft.failed) && (
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
          <CommentsPanel
            workspaceId={workspaceId}
            projectId={projectId}
            selected={selected}
            comments={commentsOnSelected}
            versionId={versionId}
            noun="node"
            compare={(comment) => ({
              then: thenOf(comment.version_id ?? ""),
              now: nowSource ? describeNode(nodeAt(nowSource, selected ?? "")) : "Loading",
            })}
            versionLabels={Object.fromEntries(choices.map((choice) => [choice.value, choice.label]))}
            onPosted={comments.reload}
          />
          <InspectPanel inspection={inspection} />
        </aside>
      </div>
    </main>
  );
}
