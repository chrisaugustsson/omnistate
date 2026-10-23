import { useState } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { ArrowLeft, Check, RotateCcw } from "lucide-react";
import { ApiError } from "@/api/client";
import { markFeatureDone, reopenFeature } from "@/api/features";
import type { FeatureBody } from "@/api/generated";
import { OverviewSections, OverviewSkeleton } from "@/components/project/overview";
import { scopedHome } from "@/components/project/items";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useHome, useProjectName } from "@/lib/use-home";

// A feature's overview: what waits for the reviewer on the feature, and its
// docs, change sets and threads, the same parts as the project's overview over
// the feature's items. It is loaded again on every live event of the project.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/$featureId/")({
  component: FeatureHome,
});

function FeatureHome() {
  const { workspaceId, projectId, featureId } = Route.useParams();
  const { home, failed, reload } = useHome(workspaceId, projectId);
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
        actions={feature && <FeatureStatus workspaceId={workspaceId} projectId={projectId} feature={feature} onChanged={reload} />}
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

/**
 * The reviewer's call on a feature: mark it done, which asks for a
 * confirmation first because the feature then takes no new change sets, or
 * reopen it.
 */
function FeatureStatus({
  workspaceId,
  projectId,
  feature,
  onChanged,
}: {
  workspaceId: string;
  projectId: string;
  feature: FeatureBody;
  onChanged: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  async function change(call: typeof markFeatureDone) {
    setBusy(true);
    setFailure(null);
    try {
      await call(workspaceId, projectId, feature.id);
      setConfirming(false);
      onChanged();
    } catch (error) {
      setFailure(
        error instanceof ApiError && error.status === 409
          ? "The feature has changed meanwhile. Reload and try again."
          : "The feature could not be changed. Try again in a moment.",
      );
    } finally {
      setBusy(false);
    }
  }

  if (feature.status === "done") {
    return (
      <>
        {failure && <span className="text-[13px] text-destructive">{failure}</span>}
        <Button variant="outline" disabled={busy} onClick={() => change(reopenFeature)}>
          <RotateCcw className="size-4" />
          Reopen
        </Button>
      </>
    );
  }
  if (!confirming) {
    return (
      <Button variant="outline" onClick={() => setConfirming(true)}>
        <Check className="size-4" />
        Mark done
      </Button>
    );
  }
  return (
    <>
      <span role="status" className="max-w-sm text-[13px] text-muted-foreground">
        {failure ?? "A done feature takes no new change sets. Its docs, threads and comments stay readable."}
      </span>
      <Button disabled={busy} onClick={() => change(markFeatureDone)}>
        Confirm
      </Button>
      <Button variant="ghost" disabled={busy} onClick={() => setConfirming(false)}>
        Cancel
      </Button>
    </>
  );
}
