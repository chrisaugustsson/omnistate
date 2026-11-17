import { createFileRoute } from "@tanstack/react-router";
import { ProjectInbox } from "@/components/home/project-inbox";
import { useProjects } from "@/lib/use-workspace";

// Home's Inbox: the comments whose latest reply is an agent's. Until S4 builds
// it across the workspace, each project's inbox as it was, one after another.
export const Route = createFileRoute("/w/$workspaceId/_home/inbox")({
  staticData: { tab: "inbox" },
  component: Inbox,
});

function Inbox() {
  const { workspaceId } = Route.useParams();
  const { projects } = useProjects(workspaceId);
  return (
    <>
      {projects?.map((project) => (
        <ProjectInbox key={project.id} workspaceId={workspaceId} projectId={project.id} />
      ))}
    </>
  );
}
