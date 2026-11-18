import { createFileRoute } from "@tanstack/react-router";
import { PartsPage } from "@/components/project/parts-list";

// The project's Parts: a row per part with its kind, what it is, its features
// and what is in flight on it; creating, renaming and deleting a part.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/")({
  staticData: { section: "parts" },
  component: Parts,
});

function Parts() {
  const { workspaceId, projectId } = Route.useParams();
  return <PartsPage workspaceId={workspaceId} projectId={projectId} />;
}
