import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import {
  ChevronRight,
  FileText,
  GitPullRequestArrow,
  Layers,
  PenTool,
  type LucideIcon,
} from "lucide-react";
import type { ChangesetBody, DocBody, HomeResponse } from "@/api/generated";
import { StatusBadge } from "@/components/status-badge";
import { timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";

// The rows that the project's pages list: docs, change sets, threads and what
// waits for the reviewer. The overview and the section pages (Inbox, Docs,
// Change sets, Threads) show the same rows.

type ProjectParams = { workspaceId: string; projectId: string };

/** The project's sections that list items: a project's own page for each, and one per feature. */
export type Section = "docs" | "changesets" | "threads";

/** The `to` and `params` of a link to a section page of the project, or of one of its features. */
export function sectionLink(section: Section, params: ProjectParams, featureId?: string | null) {
  if (featureId) {
    return {
      to: `/w/$workspaceId/projects/$projectId/features/$featureId/${section}`,
      params: { ...params, featureId },
    };
  }
  return { to: `/w/$workspaceId/projects/$projectId/${section}`, params };
}

type Scoped = { feature_id?: string | null };

/**
 * The part of a project's home that one page shows: `null` is the project's
 * own items, an ID is one feature's. The feature summaries stay as they are.
 */
export function scopedHome(home: HomeResponse, feature: string | null): HomeResponse {
  const inScope = (item: Scoped) => (item.feature_id ?? null) === feature;
  const { decisions, changesets, comments, questions } = home.waiting;
  return {
    ...home,
    docs: home.docs.filter(inScope),
    changesets: home.changesets.filter(inScope),
    threads: home.threads.filter(inScope),
    waiting: {
      decisions: decisions.filter(inScope),
      changesets: changesets.filter(inScope),
      comments: comments.filter(inScope),
      questions: questions.filter(inScope),
    },
  };
}

/** The names of the project's features by ID, for tagging the items that belong to one. */
export type FeatureNames = ReadonlyMap<string, string>;

export function featureNames(home: HomeResponse): FeatureNames {
  return new Map((home.features ?? []).map(({ feature }) => [feature.id, feature.name]));
}

/** The name of the feature that `item` belongs to, when it belongs to one and `names` is given. */
function tagOf(item: Scoped, names?: FeatureNames): string | undefined {
  return item.feature_id ? names?.get(item.feature_id) : undefined;
}

/** A small tag with the name of the feature that an item belongs to. */
export function FeatureTag({ name }: { name: string }) {
  return (
    <span className="inline-flex max-w-40 shrink-0 items-center gap-1 rounded-md px-1.5 py-px text-[11px] font-medium text-muted-foreground ring-1 ring-border ring-inset">
      <Layers aria-hidden className="size-3 shrink-0" />
      <span className="truncate">{name}</span>
    </span>
  );
}

/** The pillars of a scope's docs, in the order the pages show them. */
export const PILLARS = [
  { pillar: "spec", title: "Spec", blurb: "What gets built and why" },
  { pillar: "ui", title: "UI", blurb: "Design system and screens" },
  { pillar: "architecture", title: "Architecture", blurb: "How the system fits together" },
  { pillar: "db", title: "DB", blurb: "The data model" },
  { pillar: "research", title: "Research", blurb: "What was learned along the way" },
];

/** How many things wait for the reviewer: the comments whose latest reply is an agent's, which are all the Inbox holds. */
export function waitingCount(home: HomeResponse): number {
  return home.waiting.comments.length;
}

/** A rounded list with hairline rules between its rows. */
export function List({ children, className }: { children: ReactNode; className?: string }) {
  return <ul className={cn("divide-y overflow-hidden rounded-xl border bg-card", className)}>{children}</ul>;
}

export const rowClass =
  "group flex items-center gap-3 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none";

/** A square icon tile in front of a row. */
export function Glyph({ icon: Icon, tone }: { icon: LucideIcon; tone: string }) {
  return (
    <span className={cn("flex size-8 shrink-0 items-center justify-center rounded-lg", tone)}>
      <Icon className="size-4" />
    </span>
  );
}

function Chevron({ className }: { className?: string }) {
  return (
    <ChevronRight
      className={cn(
        "size-4 shrink-0 text-muted-foreground/50 transition-transform group-hover:translate-x-0.5 group-hover:text-foreground",
        className,
      )}
    />
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">{children}</p>
  );
}

/** A section of a page: a heading with a count, and a link to the section's own page. */
export function Section({
  id,
  title,
  count,
  more,
  children,
}: {
  id: string;
  title: string;
  count?: number;
  more?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section id={id} aria-labelledby={`${id}-title`} className="scroll-mt-6 space-y-4">
      <div className="flex items-baseline gap-2.5">
        <h2 id={`${id}-title`} className="text-[15px] font-semibold tracking-tight">
          {title}
        </h2>
        {count !== undefined && <span className="text-sm text-muted-foreground tabular-nums">{count}</span>}
        {more && <span className="ml-auto text-xs">{more}</span>}
      </div>
      {children}
    </section>
  );
}

/** "View all" to a section page, for the overview's section headings. */
export function ViewAll({ to, params }: { to: string; params: ProjectParams & { featureId?: string } }) {
  return (
    <Link
      to={to}
      params={params}
      className="inline-flex items-center gap-0.5 font-medium text-muted-foreground hover:text-foreground"
    >
      View all
      <ChevronRight className="size-3.5" />
    </Link>
  );
}

const TONES = {
  signal: "bg-signal-soft text-signal",
  amber: "bg-amber-50 text-amber-700 dark:bg-amber-400/10 dark:text-amber-300",
  sky: "bg-sky-50 text-sky-700 dark:bg-sky-400/10 dark:text-sky-300",
  violet: "bg-violet-50 text-violet-700 dark:bg-violet-400/10 dark:text-violet-300",
  plain: "bg-secondary text-muted-foreground ring-1 ring-border ring-inset",
};

/** Whether a doc is drawn on a canvas: a design doc or the design system, not a written doc. */
export function isDesign(doc: DocBody): boolean {
  return doc.kind === "design" || doc.kind === "design-system";
}

const KIND_LABELS: Record<string, string> = { html: "Doc", design: "Design", "design-system": "System" };

/** A doc's row: its kind, title, whether agents hold claims on it now. */
export function DocRow({
  doc,
  params,
  showPillar,
  names,
}: {
  doc: DocBody;
  params: ProjectParams;
  showPillar?: boolean;
  /** The project's features: given, a doc of a feature carries the feature's name. */
  names?: FeatureNames;
}) {
  const tag = tagOf(doc, names);
  const design = isDesign(doc);
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/docs/$docId"
        params={{ ...params, docId: doc.id }}
        aria-label={doc.title}
        className={rowClass}
      >
        <Glyph icon={design ? PenTool : FileText} tone={design ? TONES.violet : TONES.plain} />
        <span className="min-w-0 flex-1 truncate font-medium">{doc.title}</span>
        {tag && <FeatureTag name={tag} />}
        {doc.claims.length > 0 && (
          <span className="hidden items-center gap-1.5 text-xs text-signal sm:inline-flex">
            <span className="relative flex size-1.5">
              <span className="absolute inline-flex size-full animate-ping rounded-full bg-signal opacity-60" />
              <span className="relative inline-flex size-1.5 rounded-full bg-signal" />
            </span>
            Agents editing
          </span>
        )}
        {showPillar && (
          <span className="hidden font-mono text-[11px] text-muted-foreground uppercase sm:inline">{doc.pillar}</span>
        )}
        <span className="hidden w-12 text-right text-xs text-muted-foreground sm:inline">{KIND_LABELS[doc.kind] ?? "Doc"}</span>
        <Chevron />
      </Link>
    </li>
  );
}

/** A change set's row: its title, status and, when asked, when it was opened and its decisions. */
export function ChangesetRow({
  changeset,
  params,
  detailed,
  names,
}: {
  changeset: ChangesetBody;
  params: ProjectParams;
  detailed?: boolean;
  /** The project's features: given, a change set of a feature carries the feature's name. */
  names?: FeatureNames;
}) {
  const decisions = changeset.decisions.length;
  const tag = tagOf(changeset, names);
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/changesets/$changesetId"
        params={{ ...params, changesetId: changeset.id }}
        className={cn(rowClass, detailed && "items-start")}
      >
        <GitPullRequestArrow className={cn("size-4 shrink-0 text-muted-foreground", detailed && "mt-0.5")} />
        <span className="min-w-0 flex-1 space-y-1">
          <span className="block truncate font-medium">{changeset.title}</span>
          {detailed && (
            <span className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
              <span>Opened {timeAgo(changeset.created_at)}</span>
              {decisions > 0 && (
                <span>
                  · {decisions} {decisions === 1 ? "decision" : "decisions"}
                </span>
              )}
              {changeset.run_tag && <span className="font-mono text-[11px]">· {changeset.run_tag}</span>}
            </span>
          )}
        </span>{" "}
        {tag && <FeatureTag name={tag} />}
        <StatusBadge status={changeset.status} />
      </Link>
    </li>
  );
}
