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

/**
 * A group's band in a column list: the fill spans the column, with the rows'
 * inner padding (12px), so its icon sits over the rows' icons and its name over
 * their titles. The rows under a band are `inset` to match.
 */
export function Band({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      data-band
      className={cn(
        "flex h-(--band-h) items-center gap-2 rounded-control bg-band px-3 font-medium text-ink [&:not(:first-child)]:mt-2",
        "[&>svg:first-child]:mr-[7px] [&>svg:first-child]:ml-[3px]",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** A quiet line where a list or group has nothing; `inset` with the rows' inner padding, `indent` under a band where their titles start. */
export function EmptyRow({ children, icon, indent = false, inset = false }: { children: ReactNode; icon?: ReactNode; indent?: boolean; inset?: boolean }) {
  return (
    <div
      className={cn(
        "flex min-h-(--row-h) items-center gap-3 border-t border-line py-2.5 text-ink-3 [&_a]:font-medium [&_a]:text-ink-2 [&_a:hover]:text-ink [&_a:hover]:underline [&_a:hover]:decoration-line-strong [&_a:hover]:underline-offset-2",
        indent && "pr-3 pl-11",
        inset && !indent && "px-3",
        "[div[data-band]+&]:border-t-0",
      )}
    >
      {icon}
      <span className="min-w-0">{children}</span>
    </div>
  );
}

/**
 * The fixed columns of a track row's facts, in this order: questions, blocked,
 * open threads, part, progress, owner, when. A row keeps the width of each
 * column it lists even when it has nothing for it, so the facts line up row
 * under row and from page to page, and the last ends at the row's right edge.
 */
const SLOTS = {
  questions: "w-[88px] justify-end",
  blocked: "w-4 justify-center",
  threads: "w-9",
  part: "w-[104px]",
  progress: "w-24 justify-end",
  owner: "w-(--avatar) justify-center",
  when: "w-14 justify-end text-right",
} as const;
export type MetaSlot = keyof typeof SLOTS;

/** One column of a track row's facts. */
export function Slot({ kind, children }: { kind: MetaSlot; children?: ReactNode }) {
  return (
    <span data-col={kind} className={cn("inline-flex min-w-0 shrink-0 items-center", SLOTS[kind])}>
      {children}
    </span>
  );
}

/** A part as a chip: its mark (14px) and name, in the part column. */
export function PartChip({ part, name, parts }: { part: PartBody | undefined; name: string | undefined; parts: readonly PartBody[] | undefined }) {
  const label = part?.name ?? name;
  if (!label) return <Slot kind="part" />;
  return (
    <span data-col="part" className="inline-flex w-[104px] shrink-0 items-center gap-1.5 font-medium whitespace-nowrap text-ink-2">
      <PartIcon name={label} kind={part?.kind ?? "app"} tone={part ? partTone(parts, part.id) : 1} size="sm" />
      <span className="min-w-0 truncate">{label}</span>
    </span>
  );
}

/** "0 of 2" with a thin green bar, in the progress column, ending at its right edge; empty while the track has no stories. */
export function Progress({ summary }: { summary: TrackSummary }) {
  const stories = summary.stories;
  if (!stories || stories.total === 0) return <Slot kind="progress" />;
  const done = stories.done;
  return (
    <span
      data-col="progress"
      className="inline-flex w-24 shrink-0 items-center justify-end gap-2 text-meta whitespace-nowrap text-ink-3 tabular-nums"
      title={`${done} of ${stories.total} stories done`}
    >
      <span className="relative h-1 w-12 shrink-0 overflow-hidden rounded-full bg-line-strong">
        <i className="absolute inset-y-0 left-0 rounded-full bg-state-done" style={{ width: `${Math.round((done / stories.total) * 100)}%` }} />
      </span>
      {done} of {stories.total}
    </span>
  );
}

/**
 * The cobalt badge "3 questions", and the red blocked mark, of a track in
 * flight. In a row (`slots`) each has its column, kept when empty; elsewhere
 * (a banner) only what there is shows.
 */
export function Flags({ flags, slots = false }: { flags: TrackFlags | undefined; slots?: boolean }) {
  const questions = flags && flags.questions > 0 && (
    <span className="inline-flex h-5 shrink-0 items-center rounded-badge bg-brand-tint px-1.5 text-meta font-medium whitespace-nowrap text-brand-ink">
      {flags.questions} {flags.questions === 1 ? "question" : "questions"}
    </span>
  );
  const blocked = flags?.blocked && (
    <span title={flags.blocked} className="inline-flex shrink-0 items-center">
      <StateIcon state="blocked" label={flags.blocked} />
    </span>
  );
  if (slots) {
    return (
      <>
        <Slot kind="questions">{questions}</Slot>
        <Slot kind="blocked">{blocked}</Slot>
      </>
    );
  }
  if (!flags) return null;
  return (
    <>
      {questions}
      {blocked}
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

/**
 * A track's row: state icon, name over a grey line (or beside it), then the
 * facts in their columns: questions, blocked, part, progress and an optional
 * last column. Over two lines the grey line runs under the facts too, so it
 * truncates only at the row's end; `inset` under a band.
 */
export function TrackRow({
  workspaceId,
  summary,
  sub,
  flags,
  part,
  parts,
  two = false,
  inset = false,
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
  /** Under a band: the row has the band's inner padding, and its hover fill stays in the column. */
  inset?: boolean;
  end?: ReactNode;
}) {
  const { track } = summary;
  const facts = (
    <span className="flex h-(--avatar) shrink-0 items-center gap-3">
      <Flags flags={flags} slots />
      <PartChip part={part} name={summary.part_name} parts={parts} />
      <Progress summary={summary} />
      {end}
    </span>
  );
  return (
    <Link
      {...trackLink(workspaceId, track)}
      className={cn(
        "relative border-t border-line text-ink transition-colors duration-(--t-fast) ease-out-quint",
        "hover:bg-frame focus-visible:shadow-focus focus-visible:outline-none",
        inset ? "px-3" : "hover:shadow-[-8px_0_0_var(--frame),8px_0_0_var(--frame)]",
        "[div[data-band]+&]:border-t-0",
        two ? "grid grid-cols-[var(--avatar)_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-0.5 py-2.5" : "flex min-h-(--row-h) items-center gap-3",
      )}
    >
      <span className="grid size-(--avatar) shrink-0 place-items-center">
        <StateIcon state={track.state} />
      </span>
      {two ? (
        <>
          <span className="min-w-0 truncate font-medium">{track.name}</span>
          {sub && <span className="col-span-2 col-start-2 row-start-2 min-w-0 truncate text-ink-3">{sub}</span>}
          <span className="col-start-3 row-start-1">{facts}</span>
        </>
      ) : (
        <>
          <span className="flex min-w-0 flex-1 items-center gap-2">
            <span className="max-w-full min-w-0 shrink-0 truncate font-medium">{track.name}</span>
            {sub && <span className="min-w-0 flex-1 truncate text-ink-3">{sub}</span>}
          </span>
          {facts}
        </>
      )}
    </Link>
  );
}
