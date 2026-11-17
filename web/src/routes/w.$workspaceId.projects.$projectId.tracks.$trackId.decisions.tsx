import { useState } from "react";
import { Link, createFileRoute } from "@tanstack/react-router";
import { DecisionLog, RecordButton, RecordDecision, foldReplaced } from "@/components/track/decision-log";
import { useTrack } from "@/components/track/track-data";
import { buttonClass } from "@/components/track/ui";

// A track's Decisions: the decisions recorded on it, newest first, a replaced
// one folded under what replaced it, and for its owner the form to record one.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/tracks/$trackId/decisions")({
  component: Decisions,
});

function Decisions() {
  const { workspaceId, projectId, trackId, decisions, reloadDecisions, people, mayRecord, ownerName, features, part, summary } = useTrack();
  const [adding, setAdding] = useState(false);
  const params = { workspaceId, projectId };
  const all = decisions ?? [];
  const { top, folded } = foldReplaced(all);
  return (
    <section aria-label="Decisions">
      <div className="mb-3 flex min-h-8 flex-wrap items-center gap-x-3 gap-y-2">
        <p className="text-ink-3">
          <span className="font-medium text-ink">
            {all.length} recorded on this track
          </span>
          {all.length > 0 && (
            <>
              {" "}
              · {top.length} current{folded > 0 && ` · ${folded} replaced, folded under what replaced ${folded === 1 ? "it" : "them"}`}
            </>
          )}
        </p>
        <div className="ml-auto flex items-center gap-1">
          {summary && (
            <Link
              to="/w/$workspaceId/projects/$projectId/parts/$partId/decisions"
              params={{ ...params, partId: summary.track.part_id }}
              className={buttonClass({ variant: "ghost" })}
            >
              {summary.part_name ?? part?.name} decision log
            </Link>
          )}
          <RecordButton allowed={mayRecord} owner={ownerName} onClick={() => setAdding(true)} />
        </div>
      </div>
      {adding && mayRecord && (
        <RecordDecision
          params={params}
          trackId={trackId}
          features={features ?? []}
          part={part}
          onCancel={() => setAdding(false)}
          onDone={() => {
            setAdding(false);
            reloadDecisions();
          }}
        />
      )}
      {decisions && (
        <DecisionLog
          decisions={all}
          context={{ params, people, show: "about" }}
          empty="No decisions yet. Agents record them as questions are answered; the owner can record one here."
        />
      )}
    </section>
  );
}
