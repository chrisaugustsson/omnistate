import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

// The small pieces the feature and track pages are drawn with, in the v2 look
// (tasks/ui-redesign/v2/components.css): buttons, chips, the progress bar,
// section heads, menus and quiet empty states (dialogs: components/shell/dialog.tsx).

type ButtonLook = { variant?: "default" | "primary" | "ghost" | "danger"; size?: "default" | "sm"; icon?: boolean };

/** The class of a v2 button: 28px, a hairline and a soft shadow; primary is cobalt, ghost has no fill until hover. */
export function buttonClass({ variant = "default", size = "default", icon = false }: ButtonLook = {}, className?: string): string {
  return cn(
    "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-control border font-medium whitespace-nowrap",
    "transition-[background-color,border-color,transform] duration-(--t-fast) ease-out-quint active:scale-[0.97]",
    "focus-visible:shadow-focus focus-visible:outline-none [&_svg]:size-(--icon-sm) [&_svg]:shrink-0",
    "disabled:cursor-not-allowed disabled:border-line disabled:bg-frame disabled:text-ink-3 disabled:shadow-none disabled:active:scale-100",
    size === "sm" ? "h-6 px-2 text-meta" : "h-(--control-h) px-2.5",
    icon && (size === "sm" ? "w-6 px-0" : "w-(--control-h) px-0 [&_svg]:size-(--icon)"),
    variant === "default" && "border-line-strong bg-sheet text-ink shadow-control hover:bg-hover [&_svg]:text-ink-2",
    variant === "primary" && "border-brand bg-brand text-on-fill shadow-control hover:border-brand-hover hover:bg-brand-hover",
    variant === "danger" && "border-danger bg-danger text-on-fill hover:border-danger-hover hover:bg-danger-hover",
    variant === "ghost" &&
      "border-transparent bg-transparent text-ink-2 hover:bg-hover hover:text-ink aria-pressed:bg-selected aria-pressed:text-ink",
    className,
  );
}

/** A chip: a 24px link or label with a hairline, holding an icon and a name (a part, a feature, a track). */
export const chipClass =
  "inline-flex h-6 max-w-full shrink-0 items-center gap-1.5 rounded-control border border-line bg-sheet pr-2 pl-1.5 font-medium whitespace-nowrap text-ink transition-colors duration-(--t-fast) ease-out-quint hover:border-line-strong hover:bg-hover [&>svg]:size-(--icon-sm) [&>svg]:text-ink-2";

/** A link in running text or a panel: cobalt, underlined on hover. */
export const linkClass = "font-medium text-brand-ink underline-offset-2 hover:underline focus-visible:shadow-focus focus-visible:outline-none";

/** "n of m" with a thin green bar; `wide` lets the bar take the room it has. */
export function Progress({ done, total, wide = false, label }: { done: number; total: number; wide?: boolean; label?: ReactNode }) {
  const share = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <span className={cn("inline-flex h-6 items-center gap-2 text-meta whitespace-nowrap text-ink-3", wide && "w-full")}>
      <span aria-hidden className={cn("relative h-1 shrink-0 overflow-hidden rounded-full bg-line-strong", wide ? "flex-1" : "w-12")}>
        <i className="absolute inset-y-0 left-0 rounded-full bg-state-done" style={{ width: `${share}%` }} />
      </span>
      {label ?? `${done} of ${total}`}
    </span>
  );
}

/** A section's head: its name (15/600), a muted count, and controls on the right. */
export function SectionHead({ title, count, children, id }: { title: ReactNode; count?: ReactNode; children?: ReactNode; id?: string }) {
  return (
    <div className="mb-2 flex h-8 items-center gap-2">
      <h2 id={id} className="text-section font-semibold tracking-[-0.006em]">
        {title}
      </h2>
      {count !== undefined && <span className="font-medium text-ink-3 tabular-nums">{count}</span>}
      {children && <div className="-mr-2 ml-auto flex items-center gap-1">{children}</div>}
    </div>
  );
}

/** A quiet line in place of a list that has nothing: muted text between hairlines. */
export function EmptyLine({ children, className }: { children: ReactNode; className?: string }) {
  return <p className={cn("border-y border-line px-1 py-2.5 text-ink-3", className)}>{children}</p>;
}

/** The empty state of a tab: a short heading and one line on who fills it. */
export function EmptyTab({ title, children }: { title: ReactNode; children: ReactNode }) {
  return (
    <div className="border-y border-line py-3">
      <p className="font-medium text-ink-2">{title}</p>
      <p className="mt-0.5 max-w-[560px] text-ink-3">{children}</p>
    </div>
  );
}

export const menuClass =
  "absolute top-[calc(100%+4px)] right-0 z-40 min-w-[220px] origin-top-right animate-pop-in rounded-card bg-sheet p-1 shadow-pop";

export const menuItemClass =
  "flex h-[30px] w-full items-center gap-2 rounded-control px-2 text-left whitespace-nowrap text-ink outline-none hover:bg-hover focus-visible:bg-hover aria-disabled:cursor-not-allowed aria-disabled:text-ink-3 aria-disabled:hover:bg-transparent [&>svg]:size-(--icon-sm) [&>svg]:shrink-0 [&>svg]:text-ink-2";

/** The hint at the end of a menu item: a shortcut, or why the item is off. */
export function MenuHint({ children }: { children: ReactNode }) {
  return <span className="ml-auto pl-4 text-meta text-ink-3">{children}</span>;
}

export function MenuSeparator() {
  return <div aria-hidden className="-mx-1 my-1 h-px bg-line" />;
}

const DAY = new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "short" });
const DAY_YEAR = new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "short", year: "numeric" });

/** A date as the pages print one: "14 Nov", with the year when it is not this year's. */
export function shortDate(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  return (date.getFullYear() === now.getFullYear() ? DAY : DAY_YEAR).format(date);
}
