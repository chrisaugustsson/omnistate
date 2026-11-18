import { Link, Outlet, createFileRoute, useLocation } from "@tanstack/react-router";
import { Page, PageHead } from "@/components/shell/page";
import { partLink, useScope } from "@/components/shell/scope";
import { countsKey, useIsPhone, useReportSections } from "@/components/shell/shell-context";
import { foldReplaced } from "@/components/track/decision-log";
import { FeatureProvider, useFeatureData } from "@/components/track/feature-data";
import { openFeatureDoc } from "@/components/track/feature-docs";
import { FeatureMark, StartTrack } from "@/components/track/feature-parts";
import { linkClass } from "@/components/track/ui";
import { cn } from "@/lib/utils";

// A feature's pages. Its sections (Overview, Docs, Design only when it has
// design docs, Tracks, Decisions) are the shell's sidebar, whose counts the
// layout reports, and the header's path names the feature, so the page starts
// with the title (its mark, the name and "Start track") and then the section;
// on the phone, whose header shows the name, Start track moves under the
// Overview and the Tracks.
// Design, and Docs with a design open, are a canvas: the title keeps its place
// over a hairline that spans the window, the canvas fills the window under it,
// and the layout tells the shell so the sidebar folds. A doc opened full view
// fills the view instead (staticData.bleed). The layout loads the feature's
// data once for every section.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId")({
  staticData: { level: "feature" },
  component: FeatureLayout,
});

function FeatureLayout() {
  const { workspaceId, projectId, partId, featureId } = Route.useParams();
  const { bleed } = useScope();
  const phone = useIsPhone();
  const data = useFeatureData(workspaceId, projectId, partId, featureId);
  const [tab, docId] = useLocation({
    select: (location) => {
      const [first = "", second] = location.pathname.split(`/features/${featureId}/`)[1]?.split("/") ?? [];
      return [first, second] as const;
    },
  });
  const { feature, failed, tracks, decisions, designDocs, docs } = data;
  const current = decisions ? foldReplaced([...decisions.decisions, ...decisions.earlier], decisions.decisions).top.length : undefined;
  useReportSections(countsKey("feature", featureId), {
    docs: docs.length ? { count: docs.length, note: `${docs.length} ${docs.length === 1 ? "doc" : "docs"}` } : {},
    design: designDocs.length ? { count: designDocs.length, note: `${designDocs.length} ${designDocs.length === 1 ? "doc" : "docs"}` } : { hidden: true },
    tracks: tracks?.length ? { count: tracks.length } : {},
    decisions: current ? { count: current } : {},
  });

  // On the phone a feature's Docs with no doc named is the list alone.
  const shownDoc = tab === "docs" && (docId || !phone) ? openFeatureDoc(docs, docId) : undefined;
  const canvas = !bleed && !failed && (tab === "design" || shownDoc?.kind === "design");

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
  const column = "mx-auto w-full max-w-(--page-max) px-4 md:px-(--page-pad)";
  // On the phone the header names the feature, so the title is for screen
  // readers only, and Start track sits under the Overview and the Tracks.
  const head = phone ? (
    <h1 className="sr-only">{feature?.name ?? ""}</h1>
  ) : (
    <PageHead mark={feature && <FeatureMark />} title={feature?.name ?? ""} actions={feature && <StartTrack data={data} />} className="[&>h1]:truncate" />
  );
  return (
    <FeatureProvider value={data}>
      <main className={canvas ? "flex h-full min-h-[560px] flex-col" : "flex min-h-full flex-col"}>
        {canvas ? (
          <>
            {phone ? (
              head
            ) : (
              <div className="w-full shrink-0 border-b border-line">
                <div className={cn(column, "py-3")}>{head}</div>
              </div>
            )}
            <div className="flex min-h-0 flex-1 flex-col px-4 pt-3 pb-4">
              <Outlet />
            </div>
          </>
        ) : (
          <div className={cn(column, "flex flex-1 flex-col pt-5 pb-10 max-md:pt-4")}>
            {head}
            <div className="mt-6 flex min-w-0 flex-1 flex-col max-md:mt-0">
              <Outlet />
            </div>
          </div>
        )}
      </main>
    </FeatureProvider>
  );
}
