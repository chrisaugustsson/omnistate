import { cn } from "@/lib/utils";
import { initial } from "@/lib/identity";

// A part's mark: an app is a rounded square, a service a circle, filled in the
// part's identity colour with its initial in white. The colour is the part's
// tone (lib/identity.ts: partTones), so every screen draws a part alike.

/** The fill of each tone, as whole class names so Tailwind finds them. */
const FILLS = ["bg-part-1", "bg-part-2", "bg-part-3", "bg-part-4", "bg-part-5", "bg-part-6", "bg-part-7", "bg-part-8"];

/** The fill class of a tone (1 … 8); a tone out of range wraps. */
export function partFill(tone: number): string {
  return FILLS[(((Math.round(tone) - 1) % FILLS.length) + FILLS.length) % FILLS.length];
}

const SIZES = {
  sm: "size-(--icon-sm) text-[9px]",
  default: "size-(--icon) text-[10px]",
  md: "size-(--mark-md) text-[12px]",
  lg: "size-7 text-[15px]",
} as const;

const APP_RADIUS = { sm: "rounded-[3px]", default: "rounded-badge", md: "rounded-control", lg: "rounded-control" } as const;

export type MarkSize = keyof typeof SIZES;

/** The mark of a part: 16px in rows, 14px inside chips, 24px in the top bar (md), 28px beside a page title (lg). */
export function PartIcon({
  name,
  kind,
  tone,
  size = "default",
  className,
}: {
  name: string;
  /** "app" draws a rounded square; anything else (a service) a circle. */
  kind: string;
  /** The part's identity colour, 1 … 8: partTone(parts, part.id). */
  tone: number;
  size?: MarkSize;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      data-part-tone={tone}
      className={cn(
        "inline-grid shrink-0 place-items-center font-semibold tracking-normal text-on-fill",
        SIZES[size],
        // After the size: tailwind-merge drops a leading-* that a text-* size follows.
        "leading-none",
        kind === "app" ? APP_RADIUS[size] : "rounded-full",
        partFill(tone),
        className,
      )}
    >
      {initial(name)}
    </span>
  );
}

const PROJECT_SIZES = {
  sm: { box: "size-(--avatar) rounded-badge", grid: "grid-cols-[repeat(2,5px)] grid-rows-[repeat(2,5px)] gap-[2px]", text: "text-[10px]" },
  md: { box: "size-(--mark-md) rounded-control", grid: "grid-cols-[repeat(2,6px)] grid-rows-[repeat(2,6px)] gap-[2px]", text: "text-[11px]" },
  lg: { box: "size-7 rounded-control", grid: "grid-cols-[repeat(2,7px)] grid-rows-[repeat(2,7px)] gap-[3px]", text: "text-[13px]" },
} as const;

/**
 * A project's mark: its first three parts as small tiles in their colours on
 * ink (an app's tile square, a service's round), the fourth tile faint. A
 * project whose parts are not known (or that has none) shows its initial on a
 * neutral tile.
 */
export function ProjectMark({
  name,
  parts,
  size = "sm",
  className,
}: {
  name: string;
  /** The project's parts with their tones, oldest first; leave out while unknown. */
  parts?: readonly { kind: string; tone: number }[];
  size?: keyof typeof PROJECT_SIZES;
  className?: string;
}) {
  const measures = PROJECT_SIZES[size];
  if (!parts || parts.length === 0) {
    return (
      <span
        aria-hidden
        className={cn(
          "inline-grid shrink-0 place-items-center bg-hover font-semibold text-ink-2 ring-1 ring-line ring-inset",
          measures.box,
          measures.text,
          "leading-none",
          className,
        )}
      >
        {initial(name)}
      </span>
    );
  }
  const tiles = parts.slice(0, 3);
  return (
    <span aria-hidden className={cn("inline-grid shrink-0 place-content-center bg-ink", measures.box, measures.grid, className)}>
      {tiles.map((part, place) => (
        <i key={place} className={cn(part.kind === "app" ? "rounded-[1.5px]" : "rounded-full", partFill(part.tone))} />
      ))}
      {Array.from({ length: 4 - tiles.length }, (_, place) => (
        <i key={`rest-${place}`} className="rounded-[1.5px] bg-faint opacity-50" />
      ))}
    </span>
  );
}
