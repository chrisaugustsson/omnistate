import { createFileRoute } from "@tanstack/react-router";
import { FeaturesPage } from "@/components/project/features-list";

// A part's Features: a card per feature with what it does and the track
// changing it; creating, renaming and deleting a feature.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/")({
  staticData: { section: "features" },
  component: Features,
});

function Features() {
  const { workspaceId, projectId, partId } = Route.useParams();
  return <FeaturesPage workspaceId={workspaceId} projectId={projectId} partId={partId} />;
}
