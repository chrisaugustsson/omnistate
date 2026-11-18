import { Link, Outlet, createFileRoute, useLocation } from "@tanstack/react-router";
import { ObjectHeader } from "@/components/shell/object-header";
import { TabLink } from "@/components/shell/in-page-tabs";
import { Page } from "@/components/shell/page";
import { PathLink } from "@/components/shell/page-path";
import { partLink, useScope } from "@/components/shell/scope";
import { foldReplaced } from "@/components/track/decision-log";
import { FeatureProvider, useFeatureData } from "@/components/track/feature-data";
import { openFeatureDoc } from "@/components/track/feature-docs";
import { BeingChanged, FeatureMark, FeaturePanel, StartTrack } from "@/components/track/feature-parts";
import { linkClass } from "@/components/track/ui";

// A feature's pages. The bar keeps its part with Features lit; the page starts
// with the path (Features › the feature), the title with its mark and "Start
// track", a banner for each track that changes it now,
// and its tabs: Overview, Docs, Design (only when it has design docs), Tracks,
// Decisions. The tab's content sits beside the properties panel; Docs takes
// the full column for its list and the open doc. Design, and Docs with a design
// open, are a canvas: there the header is compact (no banners) and spans the
// window with the canvas, which fills the height under it. A doc opened full
// view fills the view instead (staticData.bleed). The layout loads the
// feature's data once for every tab.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId")({
  staticData: { scope: "feature", tab: "features" },
  component: FeatureLayout,
});

const FEATURE = "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId";

/** The tabs that want the full width, without the panel. */
const WIDE = new Set(["docs", "design"]);

function FeatureLayout() {
  const { workspaceId, projectId, partId, featureId } = Route.useParams();
  const { bleed } = useScope();
  const data = useFeatureData(workspaceId, projectId, partId, featureId);
  const [tab, docId] = useLocation({
    select: (location) => {
      const [first = "", second] = location.pathname.split(`/features/${featureId}/`)[1]?.split("/") ?? [];
      return [first, second] as const;
    },
  });
  const params = { workspaceId, projectId, partId, featureId };
  const { feature, failed, tracks, decisions, designDocs, docs } = data;

  if (bleed) {
    return (
      <FeatureProvider value={data}>
        <Outlet />
      </FeatureProvider>
    );
  }
  if (failed) {
    return (
      <Page>
        <h1 className="text-title font-semibold">Feature not found</h1>
        <p className="mt-2 text-ink-2">
          The part has no such feature, or it was deleted.{" "}
          <Link {...partLink(workspaceId, projectId, partId, "features")} className={linkClass}>
            Go to the part&rsquo;s features
          </Link>
        </p>
      </Page>
    );
  }
  const current = decisions ? foldReplaced([...decisions.decisions, ...decisions.earlier], decisions.decisions).top.length : undefined;
  const panel = !WIDE.has(tab);
  const canvas = tab === "design" || (tab === "docs" && openFeatureDoc(docs, docId)?.kind === "design");
  return (
    <FeatureProvider value={data}>
      <main className={canvas ? "flex h-full min-h-[560px] flex-col" : "flex min-h-full flex-col"}>
        <ObjectHeader
          wide={canvas}
          path={[
            <PathLink key="features" {...partLink(workspaceId, projectId, partId, "features")} activeOptions={{ exact: true }}>
              Features
            </PathLink>,
            <PathLink key="feature" to={FEATURE} params={params} activeProps={{}} aria-current="page">
              {feature?.name ?? ""}
            </PathLink>,
          ]}
          mark={feature && <FeatureMark />}
          title={feature?.name ?? ""}
          actions={feature && <StartTrack data={data} />}
          notice={canvas ? undefined : <BeingChanged data={data} />}
          tabsLabel="Feature"
          tabs={
            <>
              <TabLink to={FEATURE} params={params} activeOptions={{ exact: true }}>
                Overview
              </TabLink>
              <TabLink to={`${FEATURE}/docs`} params={params} count={docs.length || undefined}>
                Docs
              </TabLink>
              {designDocs.length > 0 && (
                <TabLink to={`${FEATURE}/design`} params={params} count={designDocs.length}>
                  Design
                </TabLink>
              )}
              <TabLink to={`${FEATURE}/tracks`} params={params} count={tracks?.length || undefined}>
                Tracks
              </TabLink>
              <TabLink to={`${FEATURE}/decisions`} params={params} count={current || undefined}>
                Decisions
              </TabLink>
            </>
          }
        />
        {canvas ? (
          <div className="flex min-h-0 flex-1 flex-col px-4 pt-3 pb-4">
            <Outlet />
          </div>
        ) : (
          <div className="mx-auto flex w-full max-w-(--page-max) flex-1 flex-col gap-12 px-4 pt-6 pb-10 md:px-(--page-pad) min-[960px]:flex-row">
            <div className="flex min-w-0 flex-1 flex-col">
              <Outlet />
            </div>
            {panel && <FeaturePanel className="w-full shrink-0 self-start min-[960px]:w-(--panel-w)" />}
          </div>
        )}
      </main>
    </FeatureProvider>
  );
}
