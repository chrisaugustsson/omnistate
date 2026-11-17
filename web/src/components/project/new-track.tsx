import { cn } from "@/lib/utils";

// A track's state as the old track pages label it. Starting a track is
// StartTrackForm (forms.tsx).

const STATE_TONES: Record<string, string> = {
  planning: "text-muted-foreground ring-border",
  building: "text-sky-700 ring-sky-300 dark:text-sky-300 dark:ring-sky-800",
  done: "text-emerald-700 ring-emerald-300 dark:text-emerald-400 dark:ring-emerald-800",
  dropped: "text-muted-foreground ring-border line-through",
};

/** A track's state as a small label. */
export function StateLabel({ state }: { state: string }) {
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-md px-1.5 py-px text-[11px] font-medium ring-1 ring-inset",
        STATE_TONES[state] ?? STATE_TONES.planning,
      )}
    >
      {state}
    </span>
  );
}
