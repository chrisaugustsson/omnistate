import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

// The properties panel: the right column of a detail page (track, feature,
// part, project), 280px with a hairline on its left, holding the facts as label
// and value rows. Sections are separated by a hairline.

/** The panel. Put it beside the main column with PageColumns. */
export function PropertiesPanel({ children, label = "Properties", className }: { children: ReactNode; label?: string; className?: string }) {
  return (
    <aside aria-label={label} className={cn("min-w-0 border-l border-line pb-4 pl-6 lg:sticky lg:top-6", className)}>
      {children}
    </aside>
  );
}

/** A group of rows, with an optional quiet heading ("Linked work"). */
export function PanelSection({ title, children, className }: { title?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn("py-3 first:pt-0 [&+&]:border-t [&+&]:border-line", className)}>
      {title && <h3 className="mb-1 flex h-6 items-center px-1 text-meta font-medium text-ink-3">{title}</h3>}
      {children}
    </section>
  );
}

/** One fact: the label on the left (84px, muted), the value on the right; `aside` is a muted line under the value. */
export function PropertyRow({ label, children, aside }: { label: ReactNode; children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="grid min-h-8 grid-cols-[84px_minmax(0,1fr)] items-start gap-2">
      <span className="truncate px-1 pt-[7px] text-ink-3">{label}</span>
      <div className="flex min-h-8 min-w-0 flex-wrap content-start items-center gap-1 py-1 [&>span:not([class])]:leading-6">
        {children}
        {aside && <span className="-mt-1 block w-full pr-1 pb-0.5 pl-0.5 text-meta text-ink-3">{aside}</span>}
      </div>
    </div>
  );
}
