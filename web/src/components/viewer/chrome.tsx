import type { ReactNode, SelectHTMLAttributes } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeft, ChevronDown, TriangleAlert } from "lucide-react";
import { AgentAvatar } from "@/components/agent-chip";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { cn } from "@/lib/utils";

// The chrome the doc and design viewers share: a slim toolbar over the view,
// compact selects, the live-draft indicator, side-pane sections with small
// uppercase labels, and alerts that float over the work.

/** The slim bar over a viewer: a back link, the doc's title and the viewer's controls on the right. */
export function ViewerToolbar({
  workspaceId,
  projectId,
  icon,
  title,
  status,
  children,
}: {
  workspaceId: string;
  projectId: string;
  icon?: ReactNode;
  /** The doc's title; `null` while it loads. */
  title: ReactNode | null;
  /** Shown after the title: the live-draft indicator. */
  status?: ReactNode;
  /** The controls on the right. */
  children?: ReactNode;
}) {
  return (
    <header className="sticky top-0 z-30 flex min-h-12 shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b bg-background/95 px-3 py-2 backdrop-blur supports-[backdrop-filter]:bg-background/80 lg:static">
      <Link
        to="/w/$workspaceId/projects/$projectId"
        params={{ workspaceId, projectId }}
        aria-label="Back to the project"
        className="-ml-1 flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-[13px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
      >
        <ArrowLeft className="size-4" />
        <span className="hidden sm:inline">Project</span>
      </Link>
      <span aria-hidden className="h-4 w-px shrink-0 bg-border" />
      <div className="flex min-w-0 flex-1 basis-48 items-center gap-2">
        {icon && <span className="flex shrink-0 text-muted-foreground [&_svg]:size-4">{icon}</span>}
        {title === null ? (
          <span aria-hidden className="h-3.5 w-40 animate-pulse rounded bg-muted" />
        ) : (
          <h1 className="min-w-0 truncate text-sm font-semibold tracking-tight">{title}</h1>
        )}
        {status}
      </div>
      {children && <div className="flex w-full flex-wrap items-center gap-2 sm:w-auto sm:flex-nowrap">{children}</div>}
    </header>
  );
}

/** A compact select with a small label in front, for the toolbar. */
export function ToolbarSelect({
  id,
  label,
  className,
  children,
  ...props
}: SelectHTMLAttributes<HTMLSelectElement> & { id: string; label: string }) {
  return (
    <div className={cn("flex min-w-0 items-center gap-1.5", className)}>
      <label htmlFor={id} className="shrink-0 text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
        {label}
      </label>
      <div className="relative min-w-0 flex-1">
        <select
          id={id}
          className="h-8 w-full min-w-0 cursor-pointer appearance-none truncate rounded-md border border-input bg-background py-0 pr-7 pl-2.5 text-[13px] shadow-xs transition-colors outline-none hover:bg-accent/60 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/30"
          {...props}
        >
          {children}
        </select>
        <ChevronDown
          aria-hidden
          className="pointer-events-none absolute top-1/2 right-2 size-3.5 -translate-y-1/2 text-muted-foreground"
        />
      </div>
    </div>
  );
}

/** A pulsing signal dot. */
export function PulseDot({ className }: { className?: string }) {
  return (
    <span aria-hidden className={cn("relative flex size-1.5 shrink-0", className)}>
      <span className="absolute inline-flex size-full animate-ping rounded-full bg-signal opacity-60" />
      <span className="relative inline-flex size-1.5 rounded-full bg-signal" />
    </span>
  );
}

/**
 * Says that a live draft is shown, with the avatars of the agents that hold
 * claims on it. The holders are named in the Claims section; here they are
 * faces only, named by their tooltips.
 */
export function LiveIndicator({ holders }: { holders: string[] }) {
  const unique = Array.from(new Set(holders));
  return (
    <span className="flex shrink-0 items-center gap-2 rounded-full bg-signal-soft py-0.5 pr-1 pl-2 text-[11px] font-medium text-signal ring-1 ring-signal/20 ring-inset">
      <PulseDot />
      <span className={unique.length ? "" : "pr-1"}>Live</span>
      {unique.length > 0 && (
        <span className="flex -space-x-1">
          {unique.slice(0, 4).map((holder) => (
            <span key={holder} title={`${holder} holds a claim`} className="rounded-full ring-2 ring-signal-soft">
              <AgentAvatar name={holder} className="size-4 text-[8px]" />
            </span>
          ))}
        </span>
      )}
    </span>
  );
}

/** A section of a side pane: a small uppercase label that sticks while its content scrolls. */
export function PaneSection({
  id,
  title,
  aside,
  className,
  bodyClassName,
  children,
}: {
  id: string;
  title: string;
  /** Next to the label, on the right: a count or a hint. */
  aside?: ReactNode;
  className?: string;
  bodyClassName?: string;
  children: ReactNode;
}) {
  return (
    <section aria-labelledby={id} className={cn("border-b last:border-b-0", className)}>
      <PaneLabel id={id} title={title} aside={aside} />
      <div className={cn("px-3 pb-3", bodyClassName)}>{children}</div>
    </section>
  );
}

/** The label row of a pane section. */
export function PaneLabel({ id, title, aside }: { id?: string; title: string; aside?: ReactNode }) {
  return (
    <div className="sticky top-0 z-10 flex h-9 items-center justify-between gap-2 bg-background/95 px-3 backdrop-blur">
      <h2 id={id} className="text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
        {title}
      </h2>
      {aside && <div className="flex items-center gap-1 text-[11px] text-muted-foreground tabular-nums">{aside}</div>}
    </div>
  );
}

/** A quiet line for a pane with nothing to show yet. */
export function PaneEmpty({ icon, children }: { icon?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-dashed px-3 py-3 text-[12.5px] leading-snug text-muted-foreground [&_svg]:mt-px [&_svg]:size-3.5 [&_svg]:shrink-0">
      {icon}
      <p>{children}</p>
    </div>
  );
}

/** Alerts that float over a viewer's work area, centred at its top. */
export function FloatingAlerts({ children }: { children: ReactNode }) {
  return (
    <div className="pointer-events-none absolute inset-x-3 top-3 z-20 flex flex-col items-center gap-2 empty:hidden">
      {children}
    </div>
  );
}

/** An error over the work, like a toast. */
export function FloatingAlert({ children }: { children: ReactNode }) {
  return (
    <Alert
      variant="destructive"
      className="pointer-events-auto flex w-auto max-w-lg items-start gap-2.5 rounded-xl border-destructive/25 bg-background/95 py-2.5 pr-4 pl-3 shadow-panel backdrop-blur"
    >
      <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
      <AlertDescription className="min-w-0 text-[13px] leading-snug break-words text-foreground">
        {children}
      </AlertDescription>
    </Alert>
  );
}
