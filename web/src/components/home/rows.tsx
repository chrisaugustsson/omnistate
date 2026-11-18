import type { ReactNode } from "react";
import { StateIcon } from "@/components/shell/state-icon";
import { cn } from "@/lib/utils";
import type { TrackFlags } from "./use-home-data";

// The small facts of a track row in the Tracks lists: its open questions and
// blocked mark, each in a fixed column.

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
