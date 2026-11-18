import { Link, Outlet, createFileRoute, useLocation } from "@tanstack/react-router";
import { ObjectHeader } from "@/components/shell/object-header";
import { Page } from "@/components/shell/page";
import { PathLink } from "@/components/shell/page-path";
import { partLink, useScope } from "@/components/shell/scope";
import { countsKey, useReportSections } from "@/components/shell/shell-context";
import { foldReplaced } from "@/components/track/decision-log";
import { FeatureProvider, useFeatureData } from "@/components/track/feature-data";
import { openFeatureDoc } from "@/components/track/feature-docs";
import { BeingChanged, FeatureMark, FeaturePanel, StartTrack } from "@/components/track/feature-parts";
import { linkClass } from "@/components/track/ui";

// A feature's pages. Its sections (Overview, Docs, Design only when it has
// design docs, Tracks, Decisions) are the shell's sidebar, whose counts the
// layout reports. The page starts with the path (Features › the feature), the
// title with its mark and "Start track", and a banner for each track that
// changes it now. The section's content sits beside the properties panel; Docs
// takes the full column for its list and the open doc. Design, and Docs with a
// design open, are a canvas: there the header keeps its place without the
// banners, and the canvas fills the window under it. A doc opened full view
// fills the view instead (staticData.bleed). The layout loads the feature's
// data once for every section.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId")({
  staticData: { level: "feature" },
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
  const current = decisions ? foldReplaced([...decisions.decisions, ...decisions.earlier], decisions.decisions).top.length : undefined;
  useReportSections(countsKey("feature", featureId), {
    docs: docs.length ? { count: docs.length, note: `${docs.length} ${docs.length === 1 ? "doc" : "docs"}` } : {},
    design: designDocs.length ? { count: designDocs.length, note: `${designDocs.length} ${designDocs.length === 1 ? "doc" : "docs"}` } : { hidden: true },
    tracks: tracks?.length ? { count: tracks.length } : {},
    decisions: current ? { count: current } : {},
  });

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
