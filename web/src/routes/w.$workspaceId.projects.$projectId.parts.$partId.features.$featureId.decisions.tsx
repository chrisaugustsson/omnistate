import { Link, createFileRoute } from "@tanstack/react-router";
import { DecisionLog, foldReplaced } from "@/components/track/decision-log";
import { useFeature } from "@/components/track/feature-data";
import { buttonClass } from "@/components/track/ui";

// A feature's Decisions: those about it that nobody replaced, newest first,
// each saying which track recorded it, a replaced one folded under what
// replaced it. Decisions are recorded on a track.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId/decisions")({
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId, partId, decisions, people, tracks, part } = useFeature();
  const { top, folded } = decisions ? foldReplaced([...decisions.decisions, ...decisions.earlier], decisions.decisions) : { top: [], folded: 0 };
  return (
    <section aria-label="Decisions">
      <div className="mb-3 flex min-h-8 flex-wrap items-center gap-x-3 gap-y-2">
        <p className="text-ink-3">
          <span className="font-medium text-ink">{top.length} current</span>
          {folded > 0 && ` · ${folded} replaced, folded under what replaced ${folded === 1 ? "it" : "them"}`}
        </p>
        <Link
          to="/w/$workspaceId/projects/$projectId/parts/$partId/decisions"
          params={{ workspaceId, projectId, partId }}
          className={buttonClass({ variant: "ghost" }, "-mr-2 ml-auto")}
        >
          {part?.name ?? "Part"} decision log
        </Link>
      </div>
      {decisions && (
        <DecisionLog
          decisions={decisions.decisions}
          earlier={decisions.earlier}
          context={{ params: { workspaceId, projectId }, people, tracks: tracks ?? [], show: "from" }}
          empty="No decisions about this feature yet. A track that changes it records them."
        />
      )}
    </section>
  );
}
