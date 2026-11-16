import { useCallback } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { loadPartDocs } from "@/api/features";
import { PageHeader } from "@/components/page-header";
import { PartDecisions } from "@/components/project/decisions";
import { FeaturesSection } from "@/components/project/features-section";
import { NewTrack } from "@/components/project/new-track";
import { KindLabel } from "@/components/project/parts-section";
import { DocRow, Empty, List } from "@/components/project/items";
import { TrackRow, orderTracks } from "@/components/project/track-row";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";
import { useParts } from "@/lib/use-parts";
import { useProjectEvents } from "@/live/context";

// A part's page: what the part is (its description), its features (the
// capabilities of the part), the docs that sit on the part itself and its tracks, the active ones first, then
// done and dropped, each group with the most recently updated first.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/parts/$partId")({
  component: PartPage,
});

function PartPage() {
  const { workspaceId, projectId, partId } = Route.useParams();
  const params = { workspaceId, projectId };
  const loadDocs = useCallback(() => loadPartDocs(workspaceId, projectId, partId), [workspaceId, projectId, partId]);
  const docs = useReloading(loadDocs);
  useProjectEvents(projectId, docs.reload);
  const { parts, failed: partsFailed } = useParts(workspaceId, projectId);
  const { home, failed: homeFailed, reload } = useHome(workspaceId, projectId);
  const part = parts?.find((candidate) => candidate.id === partId);
  const missing = parts !== null && !part;
  const tracks = (home?.tracks ?? []).filter((summary) => summary.track.part_id === partId);
  const ordered = orderTracks(tracks);

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      {(partsFailed || homeFailed || docs.failed) && (
        <Alert variant="destructive">
          <AlertDescription>The page could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {missing && (
        <div className="space-y-3">
          <h1 className="font-display text-[2.125rem] leading-[1.1] tracking-[-0.01em]">Part not found</h1>
          <p className="text-[15px] text-muted-foreground">
            The project has no such part, or it was deleted.{" "}
            <Link
              to="/w/$workspaceId/projects/$projectId/app"
              params={params}
              hash="parts"
              className="font-medium text-foreground underline underline-offset-4"
            >
              Go to the project
            </Link>
          </p>
        </div>
      )}
      {part && (
        <>
          <PageHeader
            eyebrow={
              <Link
                to="/w/$workspaceId/projects/$projectId/app"
                params={params}
                hash="parts"
                className="hover:text-foreground"
              >
                Parts
              </Link>
            }
            title={part.name}
            badge={<KindLabel kind={part.kind} />}
            description={part.description ? <p className="whitespace-pre-line">{part.description}</p> : undefined}
          />
          <div className="mt-10 space-y-12">
            <FeaturesSection workspaceId={workspaceId} projectId={projectId} partId={part.id} />
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
                      <DocRow key={doc.id} doc={doc} params={params} showPillar />
                    ))}
                  </List>
                ))}
            </section>
            <PartDecisions workspaceId={workspaceId} projectId={projectId} partId={part.id} />
            <section aria-labelledby="tracks-title" className="space-y-3">
              <div className="flex flex-wrap items-center gap-2.5">
                <h2 id="tracks-title" className="text-[15px] font-semibold tracking-tight">
                  Tracks
                </h2>
                <span className="text-sm text-muted-foreground tabular-nums">{ordered.length}</span>
              </div>
              <NewTrack {...params} fixedPartId={part.id} onCreated={reload} />
              {home &&
                (ordered.length === 0 ? (
                  <Empty>No tracks on this part yet</Empty>
                ) : (
                  <List>
                    {ordered.map((summary) => (
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
