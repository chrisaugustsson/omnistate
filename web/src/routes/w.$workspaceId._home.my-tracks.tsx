import { createFileRoute } from "@tanstack/react-router";
import { ProjectMyTracks } from "@/components/home/project-my-tracks";
import { useProjects } from "@/lib/use-workspace";

// Home's My tracks: the tracks you own. Until S4 builds it across the
// workspace, each project's list as it was, one after another.
export const Route = createFileRoute("/w/$workspaceId/_home/my-tracks")({
  staticData: { tab: "my-tracks" },
  component: MyTracks,
});

function MyTracks() {
  const { workspaceId } = Route.useParams();
  const { projects } = useProjects(workspaceId);
  return (
    <>
      {projects?.map((project) => (
        <ProjectMyTracks key={project.id} workspaceId={workspaceId} projectId={project.id} />
      ))}
    </>
  );
}
