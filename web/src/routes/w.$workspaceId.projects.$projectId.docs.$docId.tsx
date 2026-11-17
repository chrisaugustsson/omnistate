import { createFileRoute, redirect } from "@tanstack/react-router";
import { loadDoc } from "@/api/docs";
import { loadFeatures } from "@/api/features";
import { loadParts } from "@/api/parts";
import { ScopeDocs } from "@/components/project/docs-pane";
import { validateDocSearch } from "@/components/viewer/doc-page";

// A project-level doc, open in the project's Docs tab beside the list of the
// project's docs. This was the URL of every doc; a doc that belongs to a
// track, a feature or a part is sent to its place (the address keeps where in
// the doc to be) before the page shows.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/docs/$docId")({
  staticData: { tab: "docs", bleed: true },
  validateSearch: validateDocSearch,
  // Where the doc lives is read once, on the way in; the viewer then keeps the doc live.
  shouldReload: false,
  loader: async ({ params, location }) => {
    let doc;
    try {
      doc = await loadDoc(params.workspaceId, params.projectId, params.docId);
    } catch {
      return; // The viewer says the doc could not be loaded.
    }
    const search = location.search as ReturnType<typeof validateDocSearch>;
    if (doc.track_id) {
      throw redirect({ to: "/w/$workspaceId/projects/$projectId/tracks/$trackId/docs/$docId", params: { ...params, trackId: doc.track_id }, search, replace: true });
    }
    if (doc.feature_id) {
      const partId = await partOfFeature(params.workspaceId, params.projectId, doc.feature_id);
      if (partId) {
        throw redirect({
          to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/docs/$docId",
          params: { ...params, partId, featureId: doc.feature_id },
          search,
          replace: true,
        });
      }
    }
    if (doc.part_id) {
      throw redirect({ to: "/w/$workspaceId/projects/$projectId/parts/$partId/docs/$docId", params: { ...params, partId: doc.part_id }, search, replace: true });
    }
  },
  component: ProjectDoc,
});

/** The part that holds the feature `featureId`, looked up across the project's parts. */
async function partOfFeature(workspaceId: string, projectId: string, featureId: string): Promise<string | undefined> {
  try {
    const parts = await loadParts(workspaceId, projectId);
    const features = await Promise.all(parts.map((part) => loadFeatures(workspaceId, projectId, part.id)));
    return features.flat().find((feature) => feature.id === featureId)?.part_id;
  } catch {
    return undefined;
  }
}

function ProjectDoc() {
  const { workspaceId, projectId, docId } = Route.useParams();
  return <ScopeDocs workspaceId={workspaceId} projectId={projectId} docId={docId} search={Route.useSearch()} />;
}
