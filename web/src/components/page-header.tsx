import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * The top of a page: an optional line above the title (a back link or a kind),
 * the title in the display face, a description and actions on the right.
 */
export function PageHeader({
  eyebrow,
  title,
  badge,
  description,
  actions,
  className,
}: {
  eyebrow?: ReactNode;
  title: ReactNode;
  badge?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <header className={cn("flex flex-wrap items-end justify-between gap-x-6 gap-y-4", className)}>
      <div className="min-w-0 max-w-3xl space-y-2">
        {eyebrow && <div className="text-xs font-medium text-muted-foreground">{eyebrow}</div>}
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="font-display text-[2.125rem] leading-[1.1] tracking-[-0.01em] text-foreground">
            {title}
          </h1>
          {badge}
        </div>
        {description && <div className="text-[15px] leading-relaxed text-muted-foreground">{description}</div>}
      </div>
      {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}
