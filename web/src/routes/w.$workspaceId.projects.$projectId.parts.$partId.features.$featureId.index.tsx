import { useCallback } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { loadFeature, loadFeatureDocs, loadFeatureTracks } from "@/api/features";
import type { TrackSummary } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { FeatureDecisions } from "@/components/project/decisions";
import { DocRow, Empty, List } from "@/components/project/items";
import { TrackRow, orderTracks } from "@/components/project/track-row";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";
import { useParts } from "@/lib/use-parts";
import { useProjectEvents } from "@/live/context";

// A feature's page: its name and description, the docs that sit on it, and the
// tracks linked to it. The docs open in the doc viewer, the tracks on their
// own pages. It is read-only; creating, renaming and deleting happen on the
// part page, and linking in the CLI.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/")({
  component: FeaturePage,
});

function FeaturePage() {
  const { workspaceId, projectId, partId, featureId } = Route.useParams();
  const params = { workspaceId, projectId };
  const loadOne = useCallback(
    () => loadFeature(workspaceId, projectId, partId, featureId),
    [workspaceId, projectId, partId, featureId],
  );
  const loadDocs = useCallback(() => loadFeatureDocs(workspaceId, projectId, featureId), [workspaceId, projectId, featureId]);
  const loadTracks = useCallback(
    () => loadFeatureTracks(workspaceId, projectId, partId, featureId),
    [workspaceId, projectId, partId, featureId],
  );
  const one = useReloading(loadOne);
  const docs = useReloading(loadDocs);
  const linked = useReloading(loadTracks);
  useProjectEvents(projectId, () => {
    one.reload();
    docs.reload();
    linked.reload();
  });
  const { parts } = useParts(workspaceId, projectId);
  const { home } = useHome(workspaceId, projectId);
  const feature = one.value;
  const part = parts?.find((candidate) => candidate.id === partId);

  const summaries = new Map((home?.tracks ?? []).map((summary) => [summary.track.id, summary]));
  const tracks: TrackSummary[] = (linked.value ?? []).map((track) => summaries.get(track.id) ?? { track, docs: 0, waiting: 0 });

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      {one.failed && (
        <div className="space-y-3">
          <h1 className="font-display text-[2.125rem] leading-[1.1] tracking-[-0.01em]">Feature not found</h1>
          <p className="text-[15px] text-muted-foreground">
            The part has no such feature, or it was deleted.{" "}
            <Link
              to="/w/$workspaceId/projects/$projectId/parts/$partId"
              params={{ ...params, partId }}
              className="font-medium text-foreground underline underline-offset-4"
            >
              Go to the part
            </Link>
          </p>
        </div>
      )}
      {(docs.failed || linked.failed) && !one.failed && (
        <Alert variant="destructive">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {feature && (
        <>
          <PageHeader
            eyebrow={
              <Link
                to="/w/$workspaceId/projects/$projectId/parts/$partId"
                params={{ ...params, partId }}
                className="hover:text-foreground"
              >
                {part?.name ?? "Part"}
              </Link>
            }
            title={feature.name}
            description={feature.description ? <p className="whitespace-pre-line">{feature.description}</p> : undefined}
          />
          <div className="mt-10 space-y-12">
            <section aria-labelledby="docs-title" className="space-y-3">
              <div className="flex items-baseline gap-2.5">
                <h2 id="docs-title" className="text-[15px] font-semibold tracking-tight">
                  Docs
                </h2>
                <span className="text-sm text-muted-foreground tabular-nums">{docs.value?.length ?? 0}</span>
              </div>
              {docs.value &&
                (docs.value.length === 0 ? (
                  <Empty>No docs yet</Empty>
                ) : (
                  <List>
                    {docs.value.map((doc) => (
                      <DocRow key={doc.id} doc={doc} params={params} />
                    ))}
                  </List>
                ))}
            </section>
            <FeatureDecisions workspaceId={workspaceId} projectId={projectId} partId={partId} featureId={featureId} />
            <section aria-labelledby="tracks-title" className="space-y-3">
              <div className="flex items-baseline gap-2.5">
                <h2 id="tracks-title" className="text-[15px] font-semibold tracking-tight">
                  Tracks
                </h2>
                <span className="text-sm text-muted-foreground tabular-nums">{tracks.length}</span>
              </div>
              {linked.value &&
                (tracks.length === 0 ? (
                  <Empty>No tracks are linked to this feature</Empty>
                ) : (
                  <List>
                    {orderTracks(tracks).map((summary) => (
                      <TrackRow key={summary.track.id} summary={summary} params={params} />
                    ))}
                  </List>
                ))}
            </section>
          </div>
        </>
      )}
    </main>
  );
}
