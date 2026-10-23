import { Link, createFileRoute } from "@tanstack/react-router";
import { ArrowLeft } from "lucide-react";
import { OverviewSections, OverviewSkeleton } from "@/components/project/overview";
import { scopedHome } from "@/components/project/items";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { useHome, useProjectName } from "@/lib/use-home";

// A feature's overview: what waits for the reviewer on the feature, and its
// docs, change sets and threads, the same parts as the project's overview over
// the feature's items. It is loaded again on every live event of the project.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/")({
  component: FeatureHome,
});

function FeatureHome() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);
  const name = useProjectName(workspaceId, projectId);
  const feature = home?.features?.find((summary) => summary.feature.id === featureId)?.feature;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow={
          <Link
            to="/w/$workspaceId/projects/$projectId"
            params={{ workspaceId, projectId }}
            className="group inline-flex items-center gap-1.5 transition-colors hover:text-foreground"
          >
            <ArrowLeft className="size-3.5 transition-transform group-hover:-translate-x-0.5" />
            {name ?? "Project"}
          </Link>
        }
        title={feature?.name ?? (failed ? "Feature" : "")}
        badge={feature?.status === "done" ? <Badge variant="outline">Done</Badge> : undefined}
        description={feature?.description || undefined}
      />
      {failed && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The feature could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && !feature && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The project has no such feature.</AlertDescription>
        </Alert>
      )}
      {home && feature && (
        <div className="mt-10">
          <OverviewSections home={scopedHome(home, featureId)} params={{ workspaceId, projectId }} featureId={featureId} />
        </div>
      )}
      {!home && !failed && <OverviewSkeleton />}
    </main>
  );
}
