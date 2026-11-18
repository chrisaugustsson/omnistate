import { createFileRoute } from "@tanstack/react-router";
import { PlaceholderPage } from "@/components/shell/page";

// The workspace's Projects: its projects as rows (name, its parts' marks, how
// many tracks are in flight).
export const Route = createFileRoute("/w/$workspaceId/projects/")({
  staticData: { level: "workspace", section: "projects" },
  component: Projects,
});

function Projects() {
  return <PlaceholderPage title="Projects" />;
}
