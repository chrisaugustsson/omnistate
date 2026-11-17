import { cn } from "@/lib/utils";

// The 14px state icons (Linear-style, tasks/ui-redesign/v2/direction.md): a
// ring that fills as work moves, a green check when done, a grey x when
// dropped or cancelled, a red octagon when blocked. Track states and story
// statuses share the family.

type Shape = "ring" | "quarter" | "half" | "three" | "check" | "x" | "blocked";

const SHAPES: Record<string, { shape: Shape; tone: string; label: string }> = {
  // Track states
  planning: { shape: "quarter", tone: "text-state-planning", label: "Planning" },
  building: { shape: "half", tone: "text-state-building", label: "Building" },
  done: { shape: "check", tone: "text-state-done", label: "Done" },
  dropped: { shape: "x", tone: "text-state-dropped", label: "Dropped" },
  // Story statuses
  backlog: { shape: "ring", tone: "text-state-backlog", label: "Backlog" },
  implementing: { shape: "quarter", tone: "text-state-building", label: "Implementing" },
  review: { shape: "half", tone: "text-state-building", label: "Review" },
  testing: { shape: "three", tone: "text-state-building", label: "Testing" },
  cancelled: { shape: "x", tone: "text-state-dropped", label: "Cancelled" },
  // A story or track that waits on another
  blocked: { shape: "blocked", tone: "text-danger", label: "Blocked" },
};

/** The word for a state or status ("Planning", "Review"); an unknown one as it is, capitalised. */
export function stateLabel(state: string): string {
  return SHAPES[state]?.label ?? state.charAt(0).toUpperCase() + state.slice(1);
}

/** The track states in the order a track moves through them. */
export const TRACK_STATES = ["planning", "building", "done", "dropped"] as const;

/** Whether a track in `state` is in flight: planning or building (the count on a Tracks pill). */
export function inFlight(state: string): boolean {
  return state === "planning" || state === "building";
}

function Marks({ shape }: { shape: Shape }) {
  const ring = <circle cx="7" cy="7" r="5.75" fill="none" stroke="currentColor" strokeWidth="1.5" />;
  const stroke = { fill: "none", stroke: "var(--on-fill)", strokeWidth: 1.5, strokeLinecap: "round", strokeLinejoin: "round" } as const;
  switch (shape) {
    case "ring":
      return ring;
    case "quarter":
      return (
        <>
          {ring}
          <path d="M7 7V3.5A3.5 3.5 0 0 1 10.5 7Z" fill="currentColor" />
        </>
      );
    case "half":
      return (
        <>
          {ring}
          <path d="M7 3.5a3.5 3.5 0 0 1 0 7Z" fill="currentColor" />
        </>
      );
    case "three":
      return (
        <>
          {ring}
          <path d="M7 7V3.5A3.5 3.5 0 1 1 3.5 7Z" fill="currentColor" />
        </>
      );
    case "check":
      return (
        <>
          <circle cx="7" cy="7" r="6.5" fill="currentColor" />
          <path d="M4.3 7.2 6.1 9l3.6-3.9" {...stroke} />
        </>
      );
    case "x":
      return (
        <>
          <circle cx="7" cy="7" r="6.5" fill="currentColor" />
          <path d="M5 5l4 4M9 5 5 9" {...stroke} />
        </>
      );
    case "blocked":
      return (
        <>
          <path d="M4.6 .8h4.8l3.8 3.8v4.8l-3.8 3.8H4.6L.8 9.4V4.6Z" fill="currentColor" />
          <path d="M4.4 7h5.2" {...stroke} />
        </>
      );
  }
}

/**
 * The icon of a track state (planning, building, done, dropped) or a story
 * status (backlog, implementing, review, testing, done, cancelled), or
 * "blocked". It is decorative unless `label` is given: beside the state in
 * words, leave it out.
 */
export function StateIcon({ state, label, className }: { state: string; label?: string; className?: string }) {
  const { shape, tone } = SHAPES[state] ?? SHAPES.backlog;
  return (
    <svg
      viewBox="0 0 14 14"
      data-state={state}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={cn("size-(--icon-sm) shrink-0", tone, className)}
    >
      <Marks shape={shape} />
    </svg>
  );
}

/** A state in words beside its icon ("◔ Planning"); the words in green for done. */
export function StateLabel({ state, className }: { state: string; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1.5 font-medium whitespace-nowrap", state === "done" && "text-state-done-ink", className)}>
      <StateIcon state={state} />
      {stateLabel(state)}
    </span>
  );
}
