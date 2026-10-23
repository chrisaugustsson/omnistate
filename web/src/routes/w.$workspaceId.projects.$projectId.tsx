import { createFileRoute } from "@tanstack/react-router";
import { OverviewSections, OverviewSkeleton } from "@/components/project/overview";
import { waitingCount } from "@/components/project/items";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useHome, useProjectName } from "@/lib/use-home";

// The project home is the reviewer's overview of one project: its docs grouped
// by module, change sets with their status and threads, and a "waiting for
// you" section with everything that needs the human, the items of its features
// among them, tagged with the feature's name. Each part also has a page of its
// own (Inbox, Docs, Change sets, Threads), and each feature has its own
// overview. It loads the home from the API and loads it again on every live
// event of the project, so items appear and disappear without a reload.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId")({
  component: ProjectHome,
});

function ProjectHome() {
  const { workspaceId, projectId } = Route.useParams();
  const { home, failed } = useHome(workspaceId, projectId);
  const name = useProjectName(workspaceId, projectId);
  const count = home ? waitingCount(home) : 0;

  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow="Overview"
        title={name ?? "Project"}
        description={
          home &&
          (count
            ? `${count} ${count === 1 ? "thing waits" : "things wait"} for your call. Agents keep working on the rest.`
            : "You're all caught up. Agents keep working; anything that needs you shows up here.")
        }
      />
      {failed && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The project could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && (
        <div className="mt-10">
          <OverviewSections home={home} params={{ workspaceId, projectId }} />
        </div>
      )}
      {!home && !failed && <OverviewSkeleton />}
    </main>
  );
}
