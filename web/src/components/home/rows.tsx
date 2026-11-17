import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import type { PartBody, TrackSummary } from "@/api/generated";
import { PartIcon } from "@/components/shell/part-icon";
import { StateIcon } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { cn } from "@/lib/utils";
import type { TrackFlags } from "./use-home-data";

// The rows, bands and small facts of Home's pages (tasks/ui-redesign/v2:
// home.html, my-tracks.html, inbox.html): a section head, a group band, a
// track's row with its part, progress, questions and blocked mark.

/** The prototype's button: 28px, white on a hairline; `primary` in cobalt, `ghost` without a box, `sm` 24px. */
export function buttonClass({ primary = false, ghost = false, sm = false } = {}) {
  return cn(
    "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-control border font-medium whitespace-nowrap",
    "transition-[background-color,border-color,transform] duration-(--t-fast) ease-out-quint active:scale-[0.97]",
    "focus-visible:shadow-focus focus-visible:outline-none disabled:pointer-events-none disabled:bg-frame disabled:text-ink-3 disabled:shadow-none",
    sm ? "h-6 px-2 text-meta" : "h-(--control-h) px-2.5",
    primary
      ? "border-brand bg-brand text-on-fill shadow-control hover:border-brand-hover hover:bg-brand-hover"
      : ghost
        ? "border-transparent bg-transparent text-ink-2 hover:bg-hover hover:text-ink"
        : "border-line-strong bg-sheet text-ink shadow-control hover:bg-hover",
  );
}

/** A section's head: its name (15/600), a quiet count, and actions on the right. */
export function SectionHead({ id, title, count, children }: { id: string; title: string; count?: number; children?: ReactNode }) {
  return (
    <div className="mb-2 flex h-8 items-center gap-2">
      <h2 id={id} className="text-section font-semibold tracking-[-0.006em]">
        {title}
      </h2>
      {count !== undefined && <span className="font-medium text-ink-3 tabular-nums">{count}</span>}
      {children && <div className="-mr-2 ml-auto flex items-center gap-1">{children}</div>}
    </div>
  );
}

/** A list in the page column: a hairline above each row and under the last. */
export function ColumnList({ children, label, className }: { children: ReactNode; label?: string; className?: string }) {
  return (
    <div aria-label={label} className={cn("flex flex-col border-b border-line", className)}>
      {children}
    </div>
  );
}

/** A group's band in a column list: the fill reaches 8px past the column on both sides. */
export function Band({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      data-band
      className={cn(
        "-mx-2 flex h-(--band-h) items-center gap-2 rounded-control bg-band px-2 font-medium text-ink [&:not(:first-child)]:mt-2",
        "[&>svg:first-child]:mr-[7px] [&>svg:first-child]:ml-[3px]",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** A quiet line where a list or group has nothing. */
export function EmptyRow({ children, icon, indent = false }: { children: ReactNode; icon?: ReactNode; indent?: boolean }) {
  return (
    <div
      className={cn(
        "flex min-h-(--row-h) items-center gap-3 border-t border-line py-2.5 text-ink-3 [&_a]:font-medium [&_a]:text-ink-2 [&_a:hover]:text-ink [&_a:hover]:underline [&_a:hover]:decoration-line-strong [&_a:hover]:underline-offset-2",
        indent && "pl-8",
        "[div[data-band]+&]:border-t-0",
      )}
    >
      {icon}
      <span className="min-w-0">{children}</span>
    </div>
  );
}

/** A part as a chip: its mark (14px) and name, 104px wide so the columns line up row under row. */
export function PartChip({ part, name, parts }: { part: PartBody | undefined; name: string | undefined; parts: readonly PartBody[] | undefined }) {
  const label = part?.name ?? name;
  if (!label) return <span className="w-[104px] shrink-0" />;
  return (
    <span className="inline-flex w-[104px] shrink-0 items-center gap-1.5 font-medium whitespace-nowrap text-ink-2">
      <PartIcon name={label} kind={part?.kind ?? "app"} tone={part ? partTone(parts, part.id) : 1} size="sm" />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  );
}

/** "0 of 2" with a thin green bar, 96px wide; empty while the track has no stories. */
export function Progress({ summary }: { summary: TrackSummary }) {
  const stories = summary.stories;
  if (!stories || stories.total === 0) return <span className="w-24 shrink-0" />;
  const done = stories.done;
  return (
    <span
      className="inline-flex w-24 shrink-0 items-center gap-2 text-meta whitespace-nowrap text-ink-3 tabular-nums"
      title={`${done} of ${stories.total} stories done`}
    >
      <span className="relative h-1 w-12 shrink-0 overflow-hidden rounded-full bg-line-strong">
        <i className="absolute inset-y-0 left-0 rounded-full bg-state-done" style={{ width: `${Math.round((done / stories.total) * 100)}%` }} />
      </span>
      {done} of {stories.total}
    </span>
  );
}

/** The cobalt badge "3 questions", and the red blocked mark, of a track in flight. */
export function Flags({ flags }: { flags: TrackFlags | undefined }) {
  if (!flags) return null;
  return (
    <>
      {flags.questions > 0 && (
        <span className="inline-flex h-5 shrink-0 items-center rounded-badge bg-brand-tint px-1.5 text-meta font-medium whitespace-nowrap text-brand-ink">
          {flags.questions} {flags.questions === 1 ? "question" : "questions"}
        </span>
      )}
      {flags.blocked && (
        <span title={flags.blocked} className="inline-flex shrink-0 items-center">
          <StateIcon state="blocked" label={flags.blocked} />
        </span>
      )}
    </>
  );
}

/** The link to a track's page. */
export function trackLink(workspaceId: string, track: { project_id: string; id: string }) {
  return {
    to: "/w/$workspaceId/projects/$projectId/tracks/$trackId",
    params: { workspaceId, projectId: track.project_id, trackId: track.id },
  } as const;
}

/** A track's row: state icon, name over a grey line (or beside it), then flags, part, progress and an optional last column. */
export function TrackRow({
  workspaceId,
  summary,
  sub,
  flags,
  part,
  parts,
  two = false,
  end,
}: {
  workspaceId: string;
  summary: TrackSummary;
  sub: ReactNode;
  flags: TrackFlags | undefined;
  part: PartBody | undefined;
  parts: readonly PartBody[] | undefined;
  /** Title over the grey line (For you), rather than beside it (My tracks). */
  two?: boolean;
  end?: ReactNode;
}) {
  const { track } = summary;
  return (
    <Link
      {...trackLink(workspaceId, track)}
      className={cn(
        "relative flex gap-3 border-t border-line text-ink transition-colors duration-(--t-fast) ease-out-quint",
        "hover:bg-frame hover:shadow-[-8px_0_0_var(--frame),8px_0_0_var(--frame)] focus-visible:shadow-focus focus-visible:outline-none",
        "[div[data-band]+&]:border-t-0",
        two ? "items-start py-2.5" : "min-h-(--row-h) items-center",
      )}
    >
      <span className="grid size-(--avatar) shrink-0 place-items-center">
        <StateIcon state={track.state} />
      </span>
      <span className={cn("flex min-w-0 flex-1 gap-2", two ? "flex-col items-start gap-0.5" : "items-center")}>
        <span className="min-w-0 truncate font-medium">{track.name}</span>
        {sub && <span className={cn("min-w-0 truncate text-ink-3", two ? "max-w-full" : "flex-1")}>{sub}</span>}
      </span>
      <span className={cn("flex shrink-0 items-center gap-3", two && "min-h-(--avatar)")}>
        <Flags flags={flags} />
        <PartChip part={part} name={summary.part_name} parts={parts} />
        <Progress summary={summary} />
        {end}
      </span>
    </Link>
  );
}
