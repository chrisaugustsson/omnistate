import { Children, Fragment, forwardRef, type AnchorHTMLAttributes, type ReactNode } from "react";
import { createLink } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

// The small path above an object's title: its section in the bar, then the
// object, both links ("Tracks › Long time windows in Analyze"). 13/500, ink-2,
// the current step muted and truncated.

const crumb = "inline-flex h-6 items-center rounded-badge px-1.5 whitespace-nowrap";

const PathAnchor = forwardRef<HTMLAnchorElement, AnchorHTMLAttributes<HTMLAnchorElement>>(function PathAnchor(
  { className, ...props },
  ref,
) {
  return (
    <a
      ref={ref}
      {...props}
      className={cn(
        crumb,
        "text-ink-2 transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink",
        "focus-visible:shadow-focus focus-visible:outline-none",
        "aria-[current=page]:block aria-[current=page]:min-w-0 aria-[current=page]:truncate aria-[current=page]:leading-6 aria-[current=page]:text-ink-3",
        className,
      )}
    />
  );
});

/** A step of the path: a router link. The last step, the page itself, gets `aria-current="page"` from the router. */
export const PathLink = createLink(PathAnchor);

/** The path: its steps with a chevron between each. */
export function PagePath({ children, className }: { children: ReactNode; className?: string }) {
  const steps = Children.toArray(children);
  return (
    <nav aria-label="Path" className={cn("-ml-1.5 mb-2 flex min-w-0 items-center gap-0.5 font-medium", className)}>
      {steps.map((step, at) => (
        <Fragment key={at}>
          {at > 0 && <ChevronRight aria-hidden className="size-3 shrink-0 text-faint" />}
          {step}
        </Fragment>
      ))}
    </nav>
  );
}
