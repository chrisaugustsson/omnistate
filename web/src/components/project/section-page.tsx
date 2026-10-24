import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import type { HomeResponse } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { scopedHome } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useHome, useProjectName } from "@/lib/use-home";
import { cn } from "@/lib/utils";

/**
 * The frame of a project's section page (Inbox, Docs, Change sets, Threads):
 * the project's name over the section's title, and the section's content once
 * the project's home is loaded (live, as on the overview). `feature` picks the
 * part of the project the page shows: `null` its own items, an ID that
 * feature's (the name of the feature, linking to its overview, over the
 * title), and nothing everything.
 */
export function SectionPage({
  workspaceId,
  projectId,
  feature,
  title,
  description,
  children,
}: {
  workspaceId: string;
  projectId: string;
  feature?: string | null;
  title: string;
  description?: (home: HomeResponse) => ReactNode;
  children: (home: HomeResponse) => ReactNode;
}) {
  const loaded = useHome(workspaceId, projectId);
  const { failed } = loaded;
  const home = loaded.home && feature !== undefined ? scopedHome(loaded.home, feature) : loaded.home;
  const name = useProjectName(workspaceId, projectId);
  const current = feature ? loaded.home?.features?.find((summary) => summary.feature.id === feature)?.feature : undefined;
  return (
    <main className="mx-auto max-w-5xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow={
          feature ? (
            <Link
              to="/w/$workspaceId/projects/$projectId/features/$featureId"
              params={{ workspaceId, projectId, featureId: feature }}
              className="group inline-flex items-center gap-1.5 transition-colors hover:text-foreground"
            >
              <ArrowLeft className="size-3.5 transition-transform group-hover:-translate-x-0.5" />
              {current?.name ?? "Feature"}
            </Link>
          ) : (
            (name ?? "Project")
          )
        }
        title={title}
        description={home && description?.(home)}
      />
      {failed && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The project could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && <div className="mt-8 space-y-8">{children(home)}</div>}
      {!home && !failed && <Skeleton />}
    </main>
  );
}

function Skeleton() {
  return (
    <div className="mt-8 space-y-3" aria-hidden>
      {[0, 1, 2, 3].map((row) => (
        <div key={row} className="h-14 animate-pulse rounded-xl bg-muted" />
      ))}
    </div>
  );
}

/** A row of filter tabs, each with a count; the chosen one raised. */
export function FilterTabs<T extends string>({
  label,
  tabs,
  value,
  onChange,
}: {
  label: string;
  tabs: { value: T; label: string; count: number }[];
  value: T;
  onChange: (value: T) => void;
}) {
  return (
    <div role="tablist" aria-label={label} className="flex flex-wrap gap-1 border-b">
      {tabs.map((tab) => {
        const chosen = tab.value === value;
        return (
          <button
            key={tab.value}
            type="button"
            role="tab"
            aria-selected={chosen}
            onClick={() => onChange(tab.value)}
            className={cn(
              "-mb-px flex items-center gap-1.5 border-b-2 px-2.5 pb-2.5 text-[13px] font-medium transition-colors",
              chosen
                ? "border-foreground text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            {tab.label}
            <span
              className={cn(
                "rounded-full px-1.5 text-[11px] leading-[18px] tabular-nums",
                chosen ? "bg-foreground text-background" : "bg-secondary text-muted-foreground",
              )}
            >
              {tab.count}
            </span>
          </button>
        );
      })}
    </div>
  );
}
