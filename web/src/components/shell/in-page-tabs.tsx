import { forwardRef, type AnchorHTMLAttributes, type ReactNode } from "react";
import { createLink } from "@tanstack/react-router";
import { cn } from "@/lib/utils";

// An object's own sections as underline tabs in the page (feature, track):
// 36px, 13/500, ink-2; the current one ink with a 2px ink underline on the
// hairline under the row. A count follows the label, muted.

type TabProps = AnchorHTMLAttributes<HTMLAnchorElement> & { count?: ReactNode };

const TabAnchor = forwardRef<HTMLAnchorElement, TabProps>(function TabAnchor({ count, className, children, ...props }, ref) {
  return (
    <a
      ref={ref}
      {...props}
      className={cn(
        "relative inline-flex h-9 shrink-0 items-center gap-1.5 font-medium whitespace-nowrap text-ink-2",
        "transition-colors duration-(--t-fast) ease-out-quint hover:text-ink",
        "focus-visible:rounded-badge focus-visible:shadow-focus focus-visible:outline-none",
        "aria-[current=page]:text-ink aria-[current=page]:after:absolute aria-[current=page]:after:inset-x-0 aria-[current=page]:after:bottom-0",
        "aria-[current=page]:after:h-0.5 aria-[current=page]:after:rounded-t-[2px] aria-[current=page]:after:bg-ink",
        className,
      )}
    >
      {children}
      {count !== undefined && count !== null && <span className="font-medium text-ink-3 tabular-nums">{count}</span>}
    </a>
  );
});

/**
 * One tab: a router link with an optional `count`, lit while its URL is the
 * page's. Pass `activeOptions={{ exact: true }}` for a tab whose URL prefixes the
 * others' (the track's Spec at /tracks/$t), or set `aria-current="page"` yourself.
 */
export const TabLink = createLink(TabAnchor);

/**
 * The row of tabs, named for the object ("Track", "Feature"). It scrolls
 * sideways when a narrow window cannot hold every tab, never up and down, and draws no scrollbar of
 * its own, which systems that always show scrollbars would put beside the tabs.
 */
export function InPageTabs({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    <nav
      aria-label={label}
      className={cn(
        "flex items-end gap-5 overflow-x-auto overflow-y-hidden border-b border-line [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
        className,
      )}
    >
      {children}
    </nav>
  );
}
