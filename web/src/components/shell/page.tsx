import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

// The page under the bar: one centred column, 1200px wide at most with 40px
// sides, as the prototype's .wrap. A scope page starts with PageHead; an object
// page (feature, track) with PagePath, then PageHead. The main
// column and the properties panel sit side by side in PageColumns.

/** The centred page. `object` starts it closer to the bar, for the path above an object's title. */
export function Page({ children, object = false, className }: { children: ReactNode; object?: boolean; className?: string }) {
  return (
    <div className={cn("mx-auto w-full max-w-(--page-max) px-4 pb-12 md:px-(--page-pad)", object ? "pt-5" : "pt-8", className)}>
      {children}
    </div>
  );
}

/** The title row: a mark, the title (20/600), and actions on the right (one primary button at most). */
export function PageHead({ mark, title, actions, className }: { mark?: ReactNode; title?: ReactNode; actions?: ReactNode; className?: string }) {
  return (
    <div className={cn("flex min-h-(--control-h-lg) items-center gap-3", className)}>
      {mark}
      {title !== undefined && <h1 className="min-w-0 text-title font-semibold">{title}</h1>}
      {actions && <div className="ml-auto flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  );
}

/** The main column and, when given, the properties panel on the right (stacked under 960px). */
export function PageColumns({ children, panel, className }: { children: ReactNode; panel?: ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        "mt-6 grid items-start gap-12 first:mt-0",
        panel ? "grid-cols-1 min-[960px]:grid-cols-[minmax(0,1fr)_var(--panel-w)]" : "grid-cols-1",
        className,
      )}
    >
      <div className="min-w-0">{children}</div>
      {panel}
    </div>
  );
}

/** A page a later slice builds: its title and one quiet line on what it will hold. */
export function PlaceholderPage({ title, children }: { title: ReactNode; children?: ReactNode }) {
  return (
    <Page>
      <PageHead title={title} />
      {children && <p className="mt-2 max-w-(--main-max) text-em text-ink-2">{children}</p>}
    </Page>
  );
}
