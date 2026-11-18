import { createFileRoute } from "@tanstack/react-router";
import { describeFeature } from "@/api/features";
import { Intro } from "@/components/project/intro";
import { useFeature } from "@/components/track/feature-data";
import { ChangingNow, StartTrack } from "@/components/track/feature-parts";
import { useIsPhone } from "@/components/shell/shell-context";

// A feature's Overview: what it does now (its description, edited in place)
// and one line naming the tracks that change it now, each a link; on the
// phone, Start track under them.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/")({
  staticData: { section: "overview" },
  component: Overview,
});

function Overview() {
  const data = useFeature();
  const { workspaceId, projectId, partId, featureId, feature } = data;
  const phone = useIsPhone();
  if (!feature) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  return (
    <div className="max-w-(--main-max)">
      <Intro
        className="mt-0"
        text={feature.description}
        onSave={async (text) => {
          await describeFeature(workspaceId, projectId, partId, featureId, text);
        }}
      />
      <ChangingNow />
      {phone && <StartTrack data={data} className="mt-6 h-(--control-h-lg) w-full" />}
    </div>
  );
}
