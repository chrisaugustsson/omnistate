import { createFileRoute } from "@tanstack/react-router";
import { ProjectsPage } from "@/components/project/projects-list";

// The workspace's Projects: its projects as rows (the mark, the name, its
// parts' marks, how many tracks are in flight), each to the project's Overview.
export const Route = createFileRoute("/w/$workspaceId/projects/")({
  staticData: { level: "workspace", section: "projects" },
  component: Projects,
});

function Projects() {
  const { workspaceId } = Route.useParams();
  return <ProjectsPage workspaceId={workspaceId} />;
}
