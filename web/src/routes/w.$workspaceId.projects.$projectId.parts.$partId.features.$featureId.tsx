import { Outlet, createFileRoute } from "@tanstack/react-router";
import { ObjectHeader } from "@/components/shell/object-header";
import { TabLink } from "@/components/shell/in-page-tabs";
import { PathLink } from "@/components/shell/page-path";
import { partLink, useScope } from "@/components/shell/scope";
import { useFeatures } from "@/lib/use-features";

// A feature's pages. The bar keeps its part with Features lit; the page starts
// with the path (Features › the feature) and the feature's own tabs: Overview,
// Docs, Design, Tracks, Decisions. Slice S3 builds the tabs' content, the title
// row and the properties panel; until then the Overview's old page brings its
// own title. An open doc fills the view instead (staticData.bleed).
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId")({
  staticData: { scope: "feature", tab: "features" },
  component: FeatureLayout,
});

const FEATURE = "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId";

function FeatureLayout() {
  const { workspaceId, projectId, partId, featureId } = Route.useParams();
  const { bleed } = useScope();
  const { features } = useFeatures(workspaceId, projectId, partId);
  const feature = features?.find((candidate) => candidate.id === featureId);
  const params = { workspaceId, projectId, partId, featureId };
  return (
    <>
      {!bleed && (
        <ObjectHeader
          path={[
            <PathLink key="features" {...partLink(workspaceId, projectId, partId, "features")} activeOptions={{ exact: true }}>
              Features
            </PathLink>,
            <PathLink key="feature" to={FEATURE} params={params} activeProps={{}} aria-current="page">
              {feature?.name ?? ""}
            </PathLink>,
          ]}
          tabsLabel="Feature"
          tabs={
            <>
              <TabLink to={FEATURE} params={params} activeOptions={{ exact: true }}>
                Overview
              </TabLink>
              <TabLink to={`${FEATURE}/docs`} params={params}>
                Docs
              </TabLink>
              <TabLink to={`${FEATURE}/design`} params={params}>
                Design
              </TabLink>
              <TabLink to={`${FEATURE}/tracks`} params={params}>
                Tracks
              </TabLink>
              <TabLink to={`${FEATURE}/decisions`} params={params}>
                Decisions
              </TabLink>
            </>
          }
        />
      )}
      {/* The outlet keeps its place whether the header shows or not, so the tab's page is not mounted again. */}
      <Outlet />
    </>
  );
}
