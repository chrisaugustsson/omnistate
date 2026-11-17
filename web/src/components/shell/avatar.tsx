import { cn } from "@/lib/utils";
import { initial, personTone } from "@/lib/identity";

// People and agents. A person is a circle with their initial on a stable tint
// (lib/identity.ts: personTone); an agent a rounded square with a spark, on the
// pale cobalt of the prototype's tokens.

const TINTS = [
  "bg-person-1 text-person-1-ink",
  "bg-person-2 text-person-2-ink",
  "bg-person-3 text-person-3-ink",
  "bg-person-4 text-person-4-ink",
  "bg-person-5 text-person-5-ink",
  "bg-person-6 text-person-6-ink",
];

const SIZES = {
  sm: { box: "size-4 text-[8px]", spark: "size-2.5", agent: "rounded-badge" },
  default: { box: "size-(--avatar) text-[10px]", spark: "size-3", agent: "rounded-control" },
  md: { box: "size-(--mark-md) text-[11px]", spark: "size-3.5", agent: "rounded-control" },
  lg: { box: "size-7 text-[12px]", spark: "size-4", agent: "rounded-control" },
} as const;

/** The agent's spark: a four-pointed star, filled. */
function Spark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden className={className}>
      <path
        d="M12 2.8c.6 5 2.6 7.6 8.2 9.2-5.6 1.6-7.6 4.2-8.2 9.2-.6-5-2.6-7.6-8.2-9.2 5.6-1.6 7.6-4.2 8.2-9.2Z"
        fill="currentColor"
      />
    </svg>
  );
}

/**
 * A person's or an agent's avatar: 20px in rows and panels, 16px inline (sm),
 * 24px in the top bar (md). Decorative: put the name beside it, or give `title`.
 */
export function Avatar({
  name,
  agent = false,
  tintKey,
  size = "default",
  title,
  className,
}: {
  /** The name as shown ("Christopher", "writer agent"); its initial is drawn. */
  name: string;
  agent?: boolean;
  /** What the tint is chosen by, when not the name (a user ID keeps a person's tint across renames). */
  tintKey?: string;
  size?: keyof typeof SIZES;
  title?: string;
  className?: string;
}) {
  const measures = SIZES[size];
  if (agent) {
    return (
      <span
        aria-hidden
        title={title}
        className={cn(
          "inline-grid shrink-0 place-items-center bg-ink text-on-fill",
          measures.box,
          measures.agent,
          className,
        )}
      >
        <Spark className={measures.spark} />
      </span>
    );
  }
  return (
    <span
      aria-hidden
      title={title}
      className={cn(
        "inline-grid shrink-0 place-items-center rounded-full leading-none font-semibold",
        TINTS[personTone(tintKey ?? name) - 1],
        measures.box,
        className,
      )}
    >
      {initial(name)}
    </span>
  );
}
