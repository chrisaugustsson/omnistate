import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/** A keyboard key as the shell draws it: 18px, 11/500, on the hover fill. */
export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <kbd className={cn("inline-flex h-[18px] items-center rounded-badge bg-hover px-[5px] font-sans text-[11px] leading-none font-medium text-ink-3 tabular-nums", className)}>
      {children}
    </kbd>
  );
}
