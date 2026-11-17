import { Link, createFileRoute } from "@tanstack/react-router";
import { DecisionLog, foldReplaced } from "@/components/track/decision-log";
import { useFeature } from "@/components/track/feature-data";
import { SectionHead, buttonClass } from "@/components/track/ui";

// A feature's Overview: what it does now (its description) and its three
// newest current decisions, each saying which track recorded it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/")({
  component: Overview,
});

function Overview() {
  const { workspaceId, projectId, partId, featureId, feature, decisions, people, tracks } = useFeature();
  if (!feature) return <div aria-hidden className="h-40 animate-pulse rounded-card bg-frame" />;
  const current = decisions ? foldReplaced([...decisions.decisions, ...decisions.earlier], decisions.decisions).top.length : 0;
  return (
    <div className="space-y-8">
      <section aria-labelledby="now-title">
        <SectionHead id="now-title" title="What it does now" />
        {feature.description.trim() ? (
          <p className="max-w-(--main-max) text-em whitespace-pre-line text-ink">{feature.description}</p>
        ) : (
          <p className="text-ink-3">Nothing is written about what it does yet. Agents describe it when a track that changes it lands.</p>
        )}
      </section>
      <section aria-labelledby="recent-title">
        <SectionHead id="recent-title" title="Recent decisions" count={decisions && current > 3 ? `3 of ${current}` : undefined}>
          {current > 0 && (
            <Link
              to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/decisions"
              params={{ workspaceId, projectId, partId, featureId }}
              className={buttonClass({ variant: "ghost" })}
            >
              All decisions
            </Link>
          )}
        </SectionHead>
        {decisions && (
          <DecisionLog
            decisions={decisions.decisions}
            earlier={decisions.earlier}
            limit={3}
            context={{ params: { workspaceId, projectId }, people, tracks: tracks ?? [], show: "from" }}
            empty="No decisions about it yet. Tracks that change it record them."
          />
        )}
      </section>
    </div>
  );
}
