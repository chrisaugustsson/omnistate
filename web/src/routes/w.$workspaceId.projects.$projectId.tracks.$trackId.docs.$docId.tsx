import { createFileRoute } from "@tanstack/react-router";
import { DocPage, validateDocSearch } from "@/components/viewer/doc-page";

// A doc of the track in the viewer, filling the view. Slice S3 shows the
// track's docs inside its Spec, Design and How tabs.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/docs/$docId")({
  staticData: { bleed: true },
  validateSearch: validateDocSearch,
  component: TrackDoc,
});

function TrackDoc() {
  const { workspaceId, projectId, docId } = Route.useParams();
  return <DocPage workspaceId={workspaceId} projectId={projectId} docId={docId} search={Route.useSearch()} />;
}
