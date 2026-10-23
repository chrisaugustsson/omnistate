import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import {
  AlertTriangle,
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleDashed,
  CircleDot,
  FileText,
  GitMerge,
  Globe,
  GitPullRequestArrow,
  PenTool,
  Scale,
  X,
  XCircle,
  type LucideIcon,
} from "lucide-react";
import { ApiError } from "@/api/client";
import { loadBoard, loadVersionHtml, loadVersionOutline } from "@/api/docs";
import type { DecisionBody, RecordedConflict, ReviewedDoc, ReviewResponse, VersionBody } from "@/api/generated";
import { approveChangeset, loadConflicts, loadReview, rejectChangeset } from "@/api/review";
import { createAssetLoader } from "@/design/assets";
import { BoardCache, showBoardwise } from "@/design/boards";
import { createDesignRenderer, type DesignRenderer } from "@/design/renderer";
import { AgentAvatar } from "@/components/agent-chip";
import { sectionLink } from "@/components/project/items";
import { Eyebrow, PageHeader } from "@/components/page-header";
import { StatusBadge, statusWords } from "@/components/status-badge";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { TokenChanges } from "@/components/changeset/token-changes";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";
import { useHome, useProjectName } from "@/lib/use-home";
import { cn } from "@/lib/utils";
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
  "min-h-24 w-full rounded-lg border border-input bg-background px-3 py-2 text-sm shadow-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";
const SELECT_CLASS =
  "h-8 w-full appearance-none rounded-md border border-input bg-background py-1 pr-8 pl-3 text-[13px] font-medium shadow-xs outline-none hover:bg-accent/50 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50";

/** The classes of a highlight over a canvas, by the kind of change. */
const HIGHLIGHT_CLASS = {
  added: "border-emerald-600 bg-emerald-500/15",
  changed: "border-amber-500 bg-amber-400/15",
  removed: "border-red-600 bg-red-500/15",
};

/** The colours of the kinds of change: a dot, and the number in a doc's header. */
const CHANGE_TONE = {
  added: { dot: "bg-emerald-500", text: "text-emerald-700 dark:text-emerald-400", sign: "+" },
  changed: { dot: "bg-amber-500", text: "text-amber-700 dark:text-amber-400", sign: "~" },
  removed: { dot: "bg-red-500", text: "text-red-700 dark:text-red-400", sign: "−" },
};

/** A dotted drafting-table background for the design canvases. */
const CANVAS_BG =
  "bg-muted bg-[radial-gradient(circle,_color-mix(in_oklab,var(--muted-foreground)_28%,transparent)_1px,_transparent_1.2px)] [background-size:14px_14px]";

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
  const { home } = useHome(workspaceId, projectId);
  const projectName = useProjectName(workspaceId, projectId);
  const featureId = loaded?.changeset.feature_id ?? null;
  const featureName = home?.features?.find((summary) => summary.feature.id === featureId)?.feature.name;
  // Only a feature's change set can change a doc that is not its own; a doc is global when it has no feature.
  const globalDocs = new Set(featureId ? home?.docs.filter((doc) => !doc.feature_id).map((doc) => doc.id) : []);
  const globalCount = loaded?.docs.filter((doc) => globalDocs.has(doc.doc_id)).length ?? 0;

  return (
    <main className="mx-auto max-w-7xl px-5 py-8 md:px-10 md:py-12">
      <Link
        {...sectionLink("changesets", { workspaceId, projectId }, loaded?.changeset.feature_id)}
        className="group inline-flex items-center gap-1.5 rounded-md text-[13px] text-muted-foreground transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
      >
        <ArrowLeft className="size-3.5 transition-transform group-hover:-translate-x-0.5" />
        Change sets
      </Link>
      {review.failed && (
        <Alert variant="destructive" className="mt-6">
          <AlertDescription>The change set could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {loaded && (
        <>
          <Header
            review={loaded}
            scope={
              featureId ? (
                <>
                  Feature{" "}
                  <Link
                    to="/w/$workspaceId/projects/$projectId/features/$featureId"
                    params={{ workspaceId, projectId, featureId }}
                    className="font-medium text-foreground underline-offset-4 hover:underline focus-visible:underline focus-visible:outline-none"
                  >
                    {featureName ?? "…"}
                  </Link>
                </>
              ) : (
                <>
                  Project <span className="font-medium text-foreground">{projectName ?? "…"}</span>
                </>
              )
            }
          />
          <div className="mt-10 grid gap-10 lg:grid-cols-[minmax(0,1fr)_18rem] lg:gap-12">
            <div className="min-w-0 space-y-10">
              <MergeBox
                workspaceId={workspaceId}
                projectId={projectId}
                review={loaded}
                globalCount={globalCount}
                onChanged={reloadReview}
              />
              <Conflicts docs={loaded.docs} conflicts={conflicts.value} failed={conflicts.failed} />
            </div>
            <History review={loaded} />
          </div>
          <div className="mt-14">
            <Docs workspaceId={workspaceId} projectId={projectId} docs={loaded.docs} globalDocs={globalDocs} />
          </div>
        </>
      )}
      {!loaded && !review.failed && <ReviewSkeleton />}
    </main>
  );
}

function ReviewSkeleton() {
  return (
    <div className="mt-6 space-y-6" aria-hidden>
      <div className="h-10 w-2/3 animate-pulse rounded-lg bg-muted" />
      <div className="h-4 w-1/2 animate-pulse rounded bg-muted" />
      <div className="h-48 animate-pulse rounded-xl bg-muted" />
    </div>
  );
}

function Section({
  id,
  title,
  count,
  aside,
  children,
}: {
  id: string;
  title: string;
  count?: number;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className="space-y-4">
      <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2">
        <div className="flex items-baseline gap-2.5">
          <h2 id={id} className="text-[15px] font-semibold tracking-tight">
            {title}
          </h2>
          {count !== undefined && <span className="text-sm text-muted-foreground tabular-nums">{count}</span>}
        </div>
        {aside}
      </div>
      {children}
    </section>
  );
}

function Empty({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-xl border border-dashed px-4 py-6 text-center text-sm text-muted-foreground">{children}</p>
  );
}

function Header({ review, scope }: { review: ReviewResponse; scope: ReactNode }) {
  const { changeset, docs, decisions } = review;
  return (
    <PageHeader
      className="mt-5"
      eyebrow={
        <span className="inline-flex items-center gap-1.5 tracking-[0.08em] uppercase">
          <GitPullRequestArrow className="size-3.5" />
          Change set
        </span>
      }
      title={changeset.title}
      badge={<StatusBadge status={changeset.status} />}
      description={
        <div className="space-y-3">
          {changeset.description && <p>{changeset.description}</p>}
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px]">
            <span>{scope}</span>
            <span aria-hidden className="text-muted-foreground/40">
              ·
            </span>
            <span>Opened {timeAgo(changeset.created_at)}</span>
            {changeset.run_tag && (
              <>
                <span aria-hidden className="text-muted-foreground/40">
                  ·
                </span>
                <span className="rounded-md bg-secondary px-1.5 py-0.5 font-mono text-[11px] text-foreground/80 ring-1 ring-border ring-inset">
                  {changeset.run_tag}
                </span>
              </>
            )}
            <span aria-hidden className="text-muted-foreground/40">
              ·
            </span>
            <span>
              {docs.length} {docs.length === 1 ? "doc" : "docs"}
            </span>
            <span aria-hidden className="text-muted-foreground/40">
              ·
            </span>
            <span>
              {decisions.length} {decisions.length === 1 ? "decision" : "decisions"}
            </span>
          </p>
        </div>
      }
    />
  );
}

// The merge box: its state, the linked decisions and the verdict ----------------------------

type MergeState = { icon: LucideIcon; tone: string; title: ReactNode; detail?: ReactNode };

/** What the merge box says at its top, by the change set's status and its linked decisions. */
function mergeState(review: ReviewResponse, blockedId: string): MergeState {
  const { changeset, decisions } = review;
  if (changeset.status === "in_review") {
    const waiting = decisions.filter((decision) => decision.status !== "approved");
    const undecided = waiting[0];
    if (undecided) {
      return {
        icon: CircleDot,
        tone: "bg-signal-soft text-signal",
        title: (
          <span className="text-signal">
            Waiting for {waiting.length} {waiting.length === 1 ? "decision" : "decisions"}
          </span>
        ),
        detail: <span id={blockedId}>Approve opens once this decision is approved: {undecided.direction}</span>,
      };
    }
    return {
      icon: CheckCircle2,
      tone: "bg-emerald-50 text-emerald-700 dark:bg-emerald-400/10 dark:text-emerald-300",
      title: <span className="text-emerald-700 dark:text-emerald-400">Ready to approve</span>,
      detail:
        decisions.length > 0
          ? "Every linked decision is approved. Approving takes the whole change set into the project."
          : "Approving takes the whole change set into the project.",
    };
  }
  if (changeset.status === "merged") {
    return {
      icon: GitMerge,
      tone: "bg-violet-50 text-violet-700 dark:bg-violet-400/10 dark:text-violet-300",
      title: "Merged",
      detail: "The change set was approved and is part of the project.",
    };
  }
  if (changeset.status === "rejected") {
    const rejection = [...changeset.history].reverse().find((entry) => entry.to_status === "rejected")?.reason;
    return {
      icon: XCircle,
      tone: "bg-red-50 text-red-700 dark:bg-red-400/10 dark:text-red-300",
      title: "Rejected",
      detail: rejection ? `Reason: ${rejection}` : "The change set was sent back to its agents.",
    };
  }
  return {
    icon: CircleDashed,
    tone: "bg-secondary text-muted-foreground ring-1 ring-border ring-inset",
    title: <span className="capitalize">{statusWords(changeset.status)}</span>,
    detail: "Agents are still working on this change set; it is not in review yet.",
  };
}

function MergeBox({
  workspaceId,
  projectId,
  review,
  globalCount,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  review: ReviewResponse;
  /** How many of the change set's docs are global, which only a feature's change set can say. */
  globalCount: number;
  onChanged: () => void;
}) {
  const blockedId = useId();
  const state = mergeState(review, blockedId);
  const Icon = state.icon;
  return (
    <div className="overflow-hidden rounded-xl border bg-card shadow-xs">
      <div className="flex items-start gap-3 px-5 py-4">
        <span className={cn("flex size-9 shrink-0 items-center justify-center rounded-full", state.tone)}>
          <Icon className="size-[18px]" />
        </span>
        <div className="min-w-0 space-y-0.5 pt-0.5">
          <p className="text-[15px] leading-snug font-semibold tracking-tight">{state.title}</p>
          {state.detail && <p className="text-[13px] leading-relaxed text-muted-foreground">{state.detail}</p>}
        </div>
      </div>
      {globalCount > 0 && (
        <p className="flex items-center gap-2 border-t bg-amber-50 px-5 py-2.5 text-[13px] font-medium text-amber-900 dark:bg-amber-400/10 dark:text-amber-200">
          <Globe className="size-4 shrink-0" />
          {`Changes ${globalCount} global ${globalCount === 1 ? "doc" : "docs"}`}
          <span className="font-normal text-amber-800/80 dark:text-amber-200/70">
            shared by the whole project, not only this feature
          </span>
        </p>
      )}
      <Decisions workspaceId={workspaceId} projectId={projectId} decisions={review.decisions} />
      <Verdict
        workspaceId={workspaceId}
        projectId={projectId}
        review={review}
        blockedId={blockedId}
        onChanged={onChanged}
      />
    </div>
  );
}

function Decisions({
  workspaceId,
  projectId,
  decisions,
}: {
  workspaceId: string;
  projectId: string;
  decisions: DecisionBody[];
}) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="border-t">
      <h2 id={headingId} className="px-5 pt-3.5 pb-1.5 text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
        Decisions
      </h2>
      {decisions.length === 0 && <p className="px-5 pb-4 text-sm text-muted-foreground">No decision is linked.</p>}
      <ul className="divide-y divide-border/70 pb-1.5 empty:hidden">
        {decisions.map((decision) => {
          const approved = decision.status === "approved";
          return (
            <li key={decision.id} className="group relative flex items-center gap-3 px-5 py-2.5 text-sm transition-colors hover:bg-accent/60">
              {approved ? (
                <Check className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
              ) : (
                <Scale className="size-4 shrink-0 text-signal" />
              )}
              <Link
                to="/w/$workspaceId/projects/$projectId/threads/$threadId"
                params={{ workspaceId, projectId, threadId: decision.thread_id }}
                className="min-w-0 flex-1 leading-snug font-medium text-foreground after:absolute after:inset-0 focus-visible:underline focus-visible:outline-none"
              >
                {decision.direction}
              </Link>
              <StatusBadge status={decision.status} />
              <ChevronRight className="size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function Verdict({
  workspaceId,
  projectId,
  review,
  blockedId,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  review: ReviewResponse;
  /** The ID of the line that names the decision that is not approved. */
  blockedId: string;
  onChanged: () => void;
}) {
  const [rejecting, setRejecting] = useState(false);
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
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
    <section aria-label="Verdict" className="space-y-3 border-t bg-muted/50 px-5 py-4">
      {failure && (
        <Alert variant="destructive">
          <AlertDescription>{failure}</AlertDescription>
        </Alert>
      )}
      {rejecting && (
        <div className="space-y-1.5">
          <Label htmlFor={reasonId} className="text-[13px]">
            Reason
          </Label>
          <textarea
            id={reasonId}
            className={TEXTAREA_CLASS}
            placeholder="Tell the agents what to change before this can go in."
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
          <Check className="size-4" />
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
            <X className="size-4" />
            Reject
          </Button>
        )}
      </div>
    </section>
  );
}

// History --------------------------------------------------------------------------------------

function History({ review }: { review: ReviewResponse }) {
  const entries = [...review.changeset.history].reverse();
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="space-y-3 lg:pt-1">
      <Eyebrow>
        <h2 id={headingId}>History</h2>
      </Eyebrow>
      {entries.length === 0 && <p className="text-sm text-muted-foreground">No change of status yet.</p>}
      <ol className="relative space-y-4 empty:hidden">
        {entries.map((entry, index) => {
          const who = entry.actor === "human" ? "You" : entry.actor;
          return (
            <li key={`${entry.at}-${index}`} className="relative flex gap-3 text-[13px]">
              {index < entries.length - 1 && (
                <span aria-hidden className="absolute top-6 bottom-[-1rem] left-2.5 w-px bg-border" />
              )}
              <AgentAvatar name={who} className="relative" />
              <div className="min-w-0 space-y-1 pt-px">
                <p className="leading-snug">
                  <span className="font-medium">{who}</span>{" "}
                  <span className="text-muted-foreground">set it to</span>{" "}
                  <span className="font-medium">{statusWords(entry.to_status)}</span>
                </p>
                {entry.reason && <p className="text-muted-foreground">{entry.reason}</p>}
                <p className="text-xs text-muted-foreground/80">{timeAgo(entry.at)}</p>
              </div>
            </li>
          );
        })}
      </ol>
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

/** The colour key of the highlights: a dot and a word for each kind of change. */
function Legend({ kinds = ["added", "changed", "removed"] }: { kinds?: Mark["change"][] }) {
  return (
    <span className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      {kinds.map((kind) => (
        <span key={kind} className="inline-flex items-center gap-1.5 capitalize">
          <span aria-hidden className={cn("size-2 rounded-full", CHANGE_TONE[kind].dot)} />
          {kind}
        </span>
      ))}
    </span>
  );
}

function Docs({
  workspaceId,
  projectId,
  docs,
  globalDocs,
}: {
  workspaceId: string;
  projectId: string;
  docs: ReviewedDoc[];
  /** The IDs of the docs that the feature's change set shares with the whole project. */
  globalDocs: ReadonlySet<string>;
}) {
  return (
    <Section id="docs" title="Docs" count={docs.length} aside={docs.length > 0 && <Legend />}>
      {docs.length === 0 && <Empty>The change set has not saved a version of any doc.</Empty>}
      <ul className="space-y-8 empty:hidden">
        {docs.map((doc) => {
          const design = doc.kind === "design";
          return (
            <li key={doc.doc_id} className="overflow-hidden rounded-xl border bg-card shadow-xs">
              <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b bg-muted/40 px-4 py-3">
                <span
                  className={cn(
                    "flex size-8 shrink-0 items-center justify-center rounded-lg",
                    design
                      ? "bg-violet-50 text-violet-700 dark:bg-violet-400/10 dark:text-violet-300"
                      : "bg-background text-muted-foreground ring-1 ring-border ring-inset",
                  )}
                >
                  {design ? <PenTool className="size-4" /> : <FileText className="size-4" />}
                </span>
                <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2.5 gap-y-1">
                  <h3 className="truncate text-[15px] font-semibold tracking-tight">{doc.title}</h3>
                  <span className="font-mono text-[11px] text-muted-foreground uppercase">{doc.module}</span>
                  {globalDocs.has(doc.doc_id) && <Badge variant="amber">Global</Badge>}
                  {doc.removed ? (
                    <Badge variant="red">Removed doc</Badge>
                  ) : (
                    doc.base_version_id === null && <Badge variant="green">New doc</Badge>
                  )}
                </div>
                <Summary doc={doc} />
              </div>
              <div className="p-4">
                {doc.removed ? (
                  <p className="text-sm text-muted-foreground">The change set removes this doc.</p>
                ) : design ? (
                  <DesignChanges workspaceId={workspaceId} projectId={projectId} doc={doc} />
                ) : doc.kind === "html" ? (
                  <HtmlChanges workspaceId={workspaceId} projectId={projectId} doc={doc} />
                ) : doc.kind === "design-system" ? (
                  <TokenChanges
                    workspaceId={workspaceId}
                    projectId={projectId}
                    baseVersionId={doc.base_version_id ?? null}
                    versionId={doc.version_id}
                  />
                ) : null}
              </div>
            </li>
          );
        })}
      </ul>
    </Section>
  );
}

/** A doc's node changes as counts: green added, amber changed, red removed, like a diff's +/- line. */
function Summary({ doc }: { doc: ReviewedDoc }) {
  const { added, changed, moved, removed } = doc.changes;
  const counts: [Mark["change"], number][] = [
    ["added", added.length],
    ["changed", changed.length],
    ["removed", removed.length],
  ];
  return (
    <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
      {counts.map(([kind, count]) => (
        <span key={kind} className={cn("inline-flex items-baseline gap-1", count === 0 && "opacity-50")}>
          <span className={cn("font-mono text-[13px] font-medium tabular-nums", count > 0 && CHANGE_TONE[kind].text)}>
            {CHANGE_TONE[kind].sign}
            {count}
          </span>{" "}
          {kind}
        </span>
      ))}
      {moved.length > 0 && (
        <span className="inline-flex items-baseline gap-1">
          <span className="font-mono text-[13px] font-medium tabular-nums">{moved.length}</span> moved
        </span>
      )}
    </p>
  );
}

type JsonNode = Record<string, unknown>;

/** Whether the node `root` is, or holds below it, a node with the ID `id`. */
function holds(root: JsonNode, id: string): boolean {
  if (root.id === id) return true;
  return Array.isArray(root.children) && (root.children as JsonNode[]).some((child) => holds(child, id));
}

/**
 * Loads a saved design version's outline once; a saved version never changes.
 * Its boards are read one by one, by the cache this gives with the outline.
 */
function useVersion(workspaceId: string, projectId: string, docId: string, versionId: string | null) {
  const [version, setVersion] = useState<{ id: string; body: VersionBody; boards: BoardCache } | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!versionId) return;
    let cancelled = false;
    setFailed(false);
    loadVersionOutline(workspaceId, projectId, versionId).then(
      (body) => {
        const boards = new BoardCache((board) => loadBoard(workspaceId, projectId, docId, versionId, board));
        if (!cancelled) setVersion({ id: versionId, body, boards });
      },
      () => {
        if (!cancelled) setFailed(true);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [workspaceId, projectId, docId, versionId]);
  const loaded = version && version.id === versionId ? version : null;
  return { body: loaded?.body ?? null, boards: loaded?.boards ?? null, failed };
}

function DesignChanges({ workspaceId, projectId, doc }: { workspaceId: string; projectId: string; doc: ReviewedDoc }) {
  const base = useVersion(workspaceId, projectId, doc.doc_id, doc.base_version_id ?? null);
  const current = useVersion(workspaceId, projectId, doc.doc_id, doc.version_id);
  const marks = useMemo(() => marksOf(doc), [doc]);
  const [chosen, setChosen] = useState<string | null>(null);

  const currentPages = useMemo(() => current.body?.outline ?? [], [current.body]);
  const basePages = useMemo(() => base.body?.outline ?? [], [base.body]);
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
        <div className="flex items-center gap-2.5">
          <Label htmlFor={selectId} className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
            Page
          </Label>
          <div className="relative w-56">
            <select id={selectId} className={SELECT_CLASS} value={pageId} onChange={(event) => setChosen(event.target.value)}>
              {pageList.map((page) => (
                <option key={page.id} value={page.id}>
                  {page.name}
                </option>
              ))}
            </select>
            <ChevronDown
              aria-hidden
              className="pointer-events-none absolute top-1/2 right-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
            />
          </div>
        </div>
      )}
      <div className="grid gap-3 lg:grid-cols-2">
        <DesignCanvas
          workspaceId={workspaceId}
          label="Base"
          note="Before"
          kinds={["removed"]}
          empty={doc.base_version_id === null ? "The doc is new, so it has no base." : "Loading"}
          source={base.body}
          boards={base.boards}
          pageId={pageId}
          marks={marks.base}
        />
        <DesignCanvas
          workspaceId={workspaceId}
          label="New version"
          note="After"
          kinds={["added", "changed"]}
          empty="Loading"
          source={current.body}
          boards={current.boards}
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
  note,
  kinds,
  empty,
  source,
  boards,
  pageId,
  marks,
}: {
  workspaceId: string;
  label: string;
  /** A word beside the label, in the pane's header. */
  note: string;
  /** The kinds of change this side outlines, named in its header. */
  kinds: Mark["change"][];
  empty: string;
  /** The version's outline. */
  source: VersionBody | null;
  boards: BoardCache | null;
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

  const page = useMemo(() => (source?.outline ?? []).find((candidate) => String(candidate.id) === pageId) ?? null, [source, pageId]);
  const system = useMemo(() => (source ? { ...source.tokens, $components: source.components ?? [] } : null), [source]);
  const assets = useMemo(() => (renderer ? createAssetLoader(renderer, workspaceId) : null), [renderer, workspaceId]);
  useEffect(() => {
    if (!renderer || !assets || !page || !system || !boards) return;
    let cancelled = false;
    let stopBoards: (() => void) | null = null;
    // What went wrong so far, named in the canvas's message; the rest still draws.
    const problems: string[] = [];
    const report = () => setFailure(problems.length > 0 ? problems.join(" ") : null);
    // The page is shown once its images and fonts are with the renderer, with
    // its boards as placeholders; the boards in view are read and put in their place.
    void assets.load(page, system).then((loaded) => {
      if (cancelled) return;
      problems.push(...loaded);
      try {
        stopBoards = showBoardwise({
          renderer,
          page,
          system,
          cache: boards,
          prepare: async (board) => {
            problems.push(...(await assets.load(board, system)));
          },
          onShown: () => {
            setDrawn((count) => count + 1);
            report();
          },
          onFailure: (message) => {
            problems.push(message);
            report();
          },
        });
      } catch (error) {
        problems.push(error instanceof Error ? error.message : "The design could not be drawn.");
        report();
      }
    });
    return () => {
      cancelled = true;
      stopBoards?.();
    };
  }, [renderer, assets, page, system, boards]);

  const highlights = page && renderer ? marks.flatMap((mark) => highlightOf(renderer, mark)) : [];
  // Until the version is there, or when it has no such page, the canvas says why it is empty.
  const notice = source === null ? empty : page === null ? "This version has no such page." : null;

  return (
    <section aria-labelledby={headingId} className="overflow-hidden rounded-lg border bg-background">
      <div className="flex items-center justify-between gap-3 border-b px-3 py-2">
        <div className="flex items-baseline gap-2">
          <h4 id={headingId} className="text-[13px] font-semibold">
            {label}
          </h4>
          <span className="text-xs text-muted-foreground">{note}</span>
        </div>
        <span className="flex items-center gap-3">
          {marks.length > 0 && (
            <span className="font-mono text-[11px] text-muted-foreground tabular-nums">
              {marks.length} {marks.length === 1 ? "mark" : "marks"}
            </span>
          )}
          <Legend kinds={kinds} />
        </span>
      </div>
      <div className={cn("relative h-[60vh] overflow-hidden", CANVAS_BG)}>
        <canvas ref={canvas} aria-label={`${label} canvas`} className="size-full touch-none" />
        <div className="pointer-events-none absolute inset-0 overflow-hidden">{highlights}</div>
        {!failure && notice && (
          <div className="absolute inset-0 flex items-center justify-center p-6">
            <p className="max-w-xs rounded-lg border bg-background/90 px-3 py-2 text-center text-[13px] text-muted-foreground shadow-xs backdrop-blur-sm">
              {notice}
            </p>
          </div>
        )}
        {failure && (
          <p className="absolute inset-x-3 bottom-3 flex items-start gap-2 rounded-lg border border-red-600/20 bg-background/95 px-3 py-2 text-[13px] text-red-800 shadow-xs backdrop-blur-sm dark:text-red-300">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0 break-words">{failure}</span>
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
    <div className="space-y-4">
      {html.failed && (
        <Alert variant="destructive">
          <AlertDescription>The doc could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      <div className="overflow-hidden rounded-lg border">
        <div className="flex items-center justify-between gap-3 border-b bg-background px-3 py-2">
          <span className="text-[13px] font-semibold">Version under review</span>
          <Legend kinds={["added", "changed"]} />
        </div>
        <div className="bg-muted/60 p-3">
          <ViewerFrame
            workspaceId={workspaceId}
            html={html.value}
            pins={NO_PINS}
            selected={null}
            claims={NO_CLAIMS}
            changes={changes}
            onSelect={ignore}
            className="h-[50vh] w-full rounded-md bg-white shadow-xs ring-1 ring-border"
            title={`${doc.title}, the version under review`}
          />
        </div>
      </div>
      {gone.length > 0 && (
        <div className="space-y-2">
          <Eyebrow>
            <h4>Removed blocks</h4>
          </Eyebrow>
          <ul className="space-y-1.5">
            {gone.map((block) => (
              <li
                key={block.node}
                className="flex items-start gap-3 rounded-r-md border-l-2 border-red-500 bg-red-50/70 px-3 py-2 text-[13px] dark:bg-red-400/10"
              >
                <span className="shrink-0 pt-px font-mono text-[11px] text-red-700/70 dark:text-red-300/70">
                  {block.node}
                </span>
                <span className="min-w-0 text-red-800 line-through decoration-red-400/70 dark:text-red-300">
                  {block.text}
                </span>
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
  const open = (conflicts ?? []).filter((conflict) => !conflict.resolution).length;
  return (
    <Section
      id="conflicts"
      title="Conflicts"
      count={conflicts && conflicts.length > 0 ? conflicts.length : undefined}
      aside={
        open > 0 && (
          <span className="text-xs text-amber-700 dark:text-amber-400">
            {open} still {open === 1 ? "needs" : "need"} a resolution
          </span>
        )
      }
    >
      {failed && (
        <Alert variant="destructive">
          <AlertDescription>The conflicts could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {conflicts && conflicts.length === 0 && <Empty>No rebase of this change set found a conflict.</Empty>}
      <ul className="divide-y divide-amber-600/15 overflow-hidden rounded-xl border border-amber-600/25 bg-amber-50/60 empty:hidden dark:border-amber-400/20 dark:bg-amber-400/5">
        {(conflicts ?? []).map((conflict) => (
          <li key={conflict.id} className="flex items-start gap-3 px-4 py-3 text-sm">
            {conflict.resolution ? (
              <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
            ) : (
              <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
            )}
            <div className="min-w-0 flex-1 space-y-1">
              <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
                <code className="rounded bg-background/80 px-1.5 py-0.5 font-mono text-[12px] ring-1 ring-amber-600/20 ring-inset">
                  {conflict.node}
                </code>{" "}
                <span className="text-muted-foreground">
                  in <span className="text-foreground">{titleOf(conflict.doc_id)}</span> (
                  {String(conflict.kind).replace(/_/g, " ")})
                </span>
              </p>
              {conflict.resolution ? (
                <p className="text-[13px] text-foreground/80">Resolved: {conflict.resolution.note}</p>
              ) : (
                <p>
                  <Badge variant="amber">unresolved</Badge>
                </p>
              )}
            </div>
          </li>
        ))}
      </ul>
    </Section>
  );
}
