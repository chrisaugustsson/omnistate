import { useMemo, useState } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { Search } from "lucide-react";
import type { FeatureSummary } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { matches } from "@/components/palette/search";
import { Empty, List } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Input } from "@/components/ui/input";
import { timeAgo } from "@/lib/format";
import { useHome } from "@/lib/use-home";

// The Features page: every feature of the project in one flat list, the most
// recently worked on first, with a filter on name and description.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/")({
  component: Features,
});

/** A feature's last activity, or its creation while it has none. */
const activityOf = ({ feature, last_activity }: FeatureSummary) => last_activity ?? feature.created_at;

/** Most recent activity first, ties by name. */
function inOrder(features: FeatureSummary[]): FeatureSummary[] {
  return [...features].sort(
    (a, b) =>
      Date.parse(activityOf(b)) - Date.parse(activityOf(a)) ||
      a.feature.name.toLowerCase().localeCompare(b.feature.name.toLowerCase()),
  );
}

const firstLine = (text: string) => text.split(/\r?\n/, 1)[0];

function FeatureRow({ summary, params }: { summary: FeatureSummary; params: { workspaceId: string; projectId: string } }) {
  const { feature, waiting, last_activity } = summary;
  return (
    <li>
      <Link
        to="/w/$workspaceId/projects/$projectId/features/$featureId"
        params={{ ...params, featureId: feature.id }}
        className="group flex items-center gap-4 px-4 py-3 text-sm transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none"
      >
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{feature.name}</span>
          {feature.description && <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">{firstLine(feature.description)}</span>}
        </span>
        <span className="flex w-8 shrink-0 justify-end">
          {waiting > 0 && (
            <span
              title="Waiting on you"
              className="rounded-full bg-signal px-1.5 text-center text-[11px] leading-5 text-signal-foreground tabular-nums"
            >
              {waiting}
            </span>
          )}
        </span>
        <span
          title={last_activity ? new Date(last_activity).toLocaleString() : undefined}
          className="hidden w-28 shrink-0 text-right text-xs text-muted-foreground sm:inline"
        >
          {last_activity ? timeAgo(last_activity) : "No activity yet"}
        </span>
      </Link>
    </li>
  );
}

function Features() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);
  const [query, setQuery] = useState("");
  const all = useMemo(() => inOrder(home?.features ?? []), [home]);
  const shown = useMemo(
    () => (query.trim() ? all.filter(({ feature }) => matches(query, feature.name, feature.description)) : all),
    [all, query],
  );

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader title="Features" description="Every feature of the project, the most recently worked on first." />
      {failed && (
        <Alert variant="destructive" className="mt-6">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && (
        <div className="mt-6 space-y-4">
          <div className="flex items-center gap-3">
            <div className="relative max-w-md flex-1">
              <Search aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="text"
                aria-label="Filter features"
                placeholder="Filter by name or description"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                className="pl-9"
              />
            </div>
            <span className="ml-auto text-sm text-muted-foreground tabular-nums" aria-live="polite">
              {query.trim() ? `${shown.length} of ${all.length}` : `${all.length} ${all.length === 1 ? "feature" : "features"}`}
            </span>
          </div>
          {all.length === 0 ? (
            <Empty>No features yet</Empty>
          ) : shown.length === 0 ? (
            <Empty>No feature matches “{query.trim()}”. The filter looks at names and descriptions.</Empty>
          ) : (
            <List>
              {shown.map((summary) => (
                <FeatureRow key={summary.feature.id} summary={summary} params={{ workspaceId, projectId }} />
              ))}
            </List>
          )}
        </div>
      )}
    </main>
  );
}
