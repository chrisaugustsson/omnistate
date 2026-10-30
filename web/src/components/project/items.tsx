import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronRight, FileText, Layers, PenTool, type LucideIcon } from "lucide-react";
import type { DocBody, HomeResponse } from "@/api/generated";
import { cn } from "@/lib/utils";

// The rows that the project's pages list: docs, threads and what waits for the
// reviewer. The scope pages and the section pages (Inbox, Threads) show the
// same rows.

type ProjectParams = { workspaceId: string; projectId: string };

type Scoped = { feature_id?: string | null };

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

const TONES = {
  violet: "bg-violet-50 text-violet-700 dark:bg-violet-400/10 dark:text-violet-300",
  plain: "bg-secondary text-muted-foreground ring-1 ring-border ring-inset",
};

/** Whether a doc is drawn on a canvas: a design doc or the design system, not a written doc. */
export function isDesign(doc: DocBody): boolean {
  return doc.kind === "design" || doc.kind === "design-system";
}

const KIND_LABELS: Record<string, string> = { html: "Doc", design: "Design", "design-system": "System" };

/** A doc's row: its kind and title. */
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
        {showPillar && (
          <span className="hidden font-mono text-[11px] text-muted-foreground uppercase sm:inline">{doc.pillar}</span>
        )}
        <span className="hidden w-12 text-right text-xs text-muted-foreground sm:inline">{KIND_LABELS[doc.kind] ?? "Doc"}</span>
        <Chevron />
      </Link>
    </li>
  );
}
