import { forwardRef, type AnchorHTMLAttributes, type ReactNode } from "react";
import { createLink } from "@tanstack/react-router";
import { cn } from "@/lib/utils";

// The scope's sections as pills in the top bar: 28px, 13/500, radius 6. The lit
// pill has the Selected fill and ink; the others ink-2. A count follows the
// label, muted, in tabular figures.

type PillProps = AnchorHTMLAttributes<HTMLAnchorElement> & { count?: ReactNode };

const PillAnchor = forwardRef<HTMLAnchorElement, PillProps>(function PillAnchor({ count, className, children, ...props }, ref) {
  return (
    <a
      ref={ref}
      {...props}
      className={cn(
        "inline-flex h-(--pill-h) shrink-0 items-center gap-1.5 rounded-control px-2.5 font-medium whitespace-nowrap text-ink-2",
        "transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink",
        "focus-visible:shadow-focus focus-visible:outline-none",
        "aria-[current=page]:bg-selected aria-[current=page]:text-ink",
        className,
      )}
    >
      {children}
      {count !== undefined && count !== null && <span className="font-medium text-ink-3 tabular-nums">{count}</span>}
    </a>
  );
});

/**
 * One pill: a router link with an optional `count`. It is lit when it gets
 * `aria-current="page"`; the bar decides which one is (an object page lights its
 * section), so pass `activeOptions={{ exact: true }}` and `activeProps={{}}` to
 * keep the router from lighting a pill by its URL prefix.
 */
export const PillLink = createLink(PillAnchor);

/** The row of pills, named for the scope ("Monitoring platform", "Wision", "Home"). */
export function PillTabs({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <nav aria-label={label} className={cn("flex min-w-0 items-center gap-0.5", className)}>
      {children}
    </nav>
  );
}
