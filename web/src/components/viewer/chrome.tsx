import { Fragment, type ReactNode, type SelectHTMLAttributes } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronDown, ChevronRight, TriangleAlert } from "lucide-react";
import { PILLARS } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useHome } from "@/lib/use-home";
import { cn } from "@/lib/utils";

// The chrome the doc and design viewers share: a slim toolbar over the view
// with the doc's breadcrumb, compact selects, side-pane sections with small
// uppercase labels, and alerts that float over the work.

/** The part of a doc that its breadcrumb shows. */
export type CrumbedDoc = { feature_id?: string | null; pillar: string; title: string };

/**
 * The slim bar over a viewer: the doc's breadcrumb, scope › pillar › title, and
 * the viewer's controls on the right. The scope links to its page and the
 * pillar to the pillar's section on it; the app level reads "App".
 */
export function ViewerToolbar({
  workspaceId,
  projectId,
  doc,
  icon,
  children,
}: {
  workspaceId: string;
  projectId: string;
  /** The doc; `null` while it loads. */
  doc: CrumbedDoc | null;
  icon?: ReactNode;
  /** The controls on the right. */
  children?: ReactNode;
}) {
  const { home } = useHome(workspaceId, projectId);
  const featureId = doc?.feature_id ?? null;
  const scopeName = featureId
    ? (home?.features?.find((summary) => summary.feature.id === featureId)?.feature.name ?? "Feature")
    : "App";
  const scope = featureId
    ? {
        to: "/w/$workspaceId/projects/$projectId/features/$featureId",
        params: { workspaceId, projectId, featureId },
      }
    : { to: "/w/$workspaceId/projects/$projectId/app", params: { workspaceId, projectId } };
  const pillar = PILLARS.find((candidate) => candidate.pillar === doc?.pillar);
  const link =
    "block max-w-48 truncate rounded transition-colors hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none";
  return (
    <header className="sticky top-0 z-30 flex min-h-12 shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b bg-background/95 px-3 py-2 backdrop-blur supports-[backdrop-filter]:bg-background/80 lg:static">
      <nav aria-label="Breadcrumb" className="min-w-0 flex-1 basis-48">
        {doc === null ? (
          <span aria-hidden className="block h-3.5 w-40 animate-pulse rounded bg-muted" />
        ) : (
          <ol className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5 text-xs font-medium text-muted-foreground">
            <li className="min-w-0">
              <Link {...scope} className={link}>
                {scopeName}
              </Link>
            </li>
            <Crumb>
              <Link {...scope} hash={doc.pillar} className={link}>
                {pillar?.title ?? doc.pillar}
              </Link>
            </Crumb>
            <Crumb>
              <span className="flex min-w-0 items-center gap-2 text-foreground [&_svg]:size-4 [&_svg]:text-muted-foreground">
                {icon}
                <h1 aria-current="page" className="min-w-0 truncate text-sm font-semibold tracking-tight">
                  {doc.title}
                </h1>
              </span>
            </Crumb>
          </ol>
        )}
      </nav>
      {children && <div className="flex w-full flex-wrap items-center gap-2 sm:w-auto sm:flex-nowrap">{children}</div>}
    </header>
  );
}

/** A crumb after the first: a chevron and the crumb. */
function Crumb({ children }: { children: ReactNode }) {
  return (
    <Fragment>
      <ChevronRight aria-hidden className="size-3 shrink-0 text-muted-foreground/50" />
      <li className="min-w-0">{children}</li>
    </Fragment>
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
