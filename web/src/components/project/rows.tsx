import { forwardRef, type AnchorHTMLAttributes, type ReactNode } from "react";
import { createLink } from "@tanstack/react-router";
import { Page } from "@/components/shell/page";
import { cn } from "@/lib/utils";

// The pieces the project's and the part's pages are built of, after the
// prototype (tasks/ui-redesign/v2/components.css): buttons and fields, a
// section with its head, rows on the sheet with a hairline between them, group
// bands, a progress bar, badges and the panel's counts.

/** A page of a scope (the project's or a part's tabs): the main landmark around the centred page. */
export function ScopePage({ children }: { children: ReactNode }) {
  return (
    <main>
      <Page>{children}</Page>
    </main>
  );
}

/** A secondary button: white with a hairline and a tiny shadow, 28px. */
export const btn =
  "inline-flex h-(--control-h) shrink-0 items-center justify-center gap-1.5 rounded-control border border-line-strong bg-sheet px-2.5 font-medium whitespace-nowrap text-ink shadow-control " +
  "transition-[background-color,border-color,transform] duration-(--t-fast) ease-out-quint hover:bg-hover active:scale-[0.97] " +
  "focus-visible:shadow-focus focus-visible:outline-none disabled:cursor-not-allowed disabled:bg-frame disabled:text-ink-3 disabled:shadow-none disabled:active:scale-100 [&_svg]:size-(--icon-sm) [&_svg]:text-ink-2";

/** The one primary button of a screen: cobalt. */
export const btnPrimary = cn(
  btn,
  "border-brand bg-brand text-on-fill hover:border-brand-hover hover:bg-brand-hover [&_svg]:text-on-fill disabled:border-line-strong",
);

/** A quiet button: no fill until hovered. */
export const btnGhost = cn(btn, "border-transparent bg-transparent text-ink-2 shadow-none hover:text-ink");

/** A small quiet button (a section's "All tracks"): 24px, 12px type. */
export const btnGhostSm = cn(btnGhost, "h-6 px-2 text-meta");

/** A square icon button. */
export const btnIcon = cn(btnGhost, "w-(--control-h) px-0 [&_svg]:size-(--icon-sm)");

/** A danger button. */
export const btnDanger = cn(btn, "border-danger bg-danger text-on-fill hover:border-danger-hover hover:bg-danger-hover");

/** A text field or select, 32px. */
export const field =
  "h-(--control-h-lg) w-full min-w-0 rounded-control border border-line-strong bg-sheet px-2.5 text-ink transition-[border-color,box-shadow] duration-(--t-fast) " +
  "placeholder:text-ink-3 hover:border-faint focus:border-brand focus:shadow-focus focus:outline-none";

/** A section of a page: its head (title, count, an action at the end) and its content. */
export function Section({
  id,
  title,
  count,
  end,
  children,
  className,
}: {
  id: string;
  title: ReactNode;
  count?: ReactNode;
  end?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section aria-labelledby={id} className={cn("mt-8", className)}>
      <div className="mb-2 flex h-8 items-center gap-2">
        <h2 id={id} className="text-section font-semibold tracking-[-0.006em]">
          {title}
        </h2>
        {count !== undefined && <span className="font-medium text-ink-3 tabular-nums">{count}</span>}
        {end && <div className="-mr-2 ml-auto flex items-center gap-1">{end}</div>}
      </div>
      {children}
    </section>
  );
}

/** Rows in the page's column: a hairline above each row and under the last. */
export function ColumnList({ children, label }: { children: ReactNode; label?: string }) {
  return (
    <ul aria-label={label} className="flex flex-col border-b border-line">
      {children}
    </ul>
  );
}

/** The classes of a row: 40px, an icon slot, a title, trailing facts; `two` for a title over a grey line. */
export function rowClass({ two = false, link = false, inset = false }: { two?: boolean; link?: boolean; inset?: boolean } = {}) {
  return cn(
    "relative flex min-h-(--row-h) gap-3 border-t border-line text-ink",
    two ? "items-start py-2.5" : "items-center",
    inset ? "px-3" : "px-0",
    link &&
      "transition-colors duration-(--t-fast) ease-out-quint hover:bg-frame focus-visible:bg-frame focus-visible:outline-none " +
        (inset ? "" : "hover:shadow-[-8px_0_0_var(--frame),8px_0_0_var(--frame)] focus-visible:shadow-[-8px_0_0_var(--frame),8px_0_0_var(--frame)]"),
  );
}

/** A row's icon slot, 20px wide, centred. */
export function RowLead({ children, two = false }: { children: ReactNode; two?: boolean }) {
  return <span className={cn("grid w-(--avatar) shrink-0 place-items-center", two && "h-5")}>{children}</span>;
}

/** A row that says there is nothing, quietly. */
export function EmptyRow({ children, icon, inset = false }: { children: ReactNode; icon?: ReactNode; inset?: boolean }) {
  return (
    <li className={cn("flex min-h-(--row-h) items-center gap-3 border-t border-line py-2.5 text-ink-3", inset && "px-3", " [&_a]:font-medium [&_a]:text-ink-2 [&_a:hover]:text-ink [&_a:hover]:underline")}>
      {icon && <RowLead>{icon}</RowLead>}
      <span>{children}</span>
    </li>
  );
}

/** A group band over rows: a pale tinted bar with an icon, the group's name and its count. */
export function Band({ icon, children, count, end }: { icon?: ReactNode; children: ReactNode; count?: number; end?: ReactNode }) {
  return (
    <div role="presentation" className="flex h-(--band-h) items-center gap-2 rounded-control bg-band pr-3 pl-3 font-medium text-ink">
      {icon && <span className="mr-[7px] ml-[3px] grid place-items-center">{icon}</span>}
      {children}
      {count !== undefined && <span className="font-medium text-ink-3 tabular-nums">{count}</span>}
      {end && <div className="ml-auto flex items-center gap-1">{end}</div>}
    </div>
  );
}

/** Done of total as a thin green bar and "n of m". */
export function Progress({ done, total, className }: { done: number; total: number; className?: string }) {
  if (total === 0) return <span aria-hidden className={cn("inline-block", className)} />;
  const value = total > 0 ? Math.round((done / total) * 100) : 0;
  return (
    <span
      className={cn("inline-flex items-center gap-2 text-meta whitespace-nowrap text-ink-3 tabular-nums", className)}
      title={`${done} of ${total} stories done`}
    >
      <span aria-hidden className="relative h-1 w-12 shrink-0 overflow-hidden rounded-full bg-line-strong">
        <i className="absolute inset-y-0 left-0 rounded-full bg-state-done" style={{ width: `${value}%` }} />
      </span>
      {done} of {total}
    </span>
  );
}

/** A small badge: grey, or the brand's tint. */
export function Badge({ children, tone = "plain", className }: { children: ReactNode; tone?: "plain" | "brand"; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center gap-1 rounded-badge px-1.5 text-meta font-medium whitespace-nowrap",
        tone === "brand" ? "bg-brand-tint text-brand-ink" : "bg-hover text-ink-2",
        className,
      )}
    >
      {children}
    </span>
  );
}

const CountAnchor = forwardRef<HTMLAnchorElement, AnchorHTMLAttributes<HTMLAnchorElement> & { n: number; label: string }>(
  function CountAnchor({ n, label, className, ...props }, ref) {
    return (
      <a
        ref={ref}
        {...props}
        className={cn(
          "flex flex-col rounded-control px-1 py-1.5 leading-[1.3] transition-colors duration-(--t-fast) hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none",
          className,
        )}
      >
        <span className="text-section font-semibold text-ink tabular-nums">{n}</span>
        <span className="text-meta text-ink-3">{label}</span>
      </a>
    );
  },
);

/** One of the panel's counts, a link to where the counted things are. */
export const CountLink = createLink(CountAnchor);

/** The panel's counts, three to a row. */
export function Counts({ children }: { children: ReactNode }) {
  return <div className="grid grid-cols-3 gap-0.5">{children}</div>;
}

/** A short date: "12 min", "3 h", "4 Nov"; for the time column of a row. */
export function shortWhen(iso: string, now = Date.now()): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const minutes = Math.max(0, Math.round((now - then) / 60000));
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h`;
  return shortDate(iso);
}

/** A day as "4 Nov". */
export function shortDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

/** The muted dot between facts on a meta line. */
export function Sep() {
  return <span className="text-faint"> · </span>;
}
