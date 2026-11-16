import { useCallback, useState } from "react";
import { ApiError } from "@/api/client";
import { loadTrackDecisions, loadFeatureDecisions, recordDecision } from "@/api/decisions";
import { loadTrackFeatures } from "@/api/features";
import type { DecisionBody } from "@/api/generated";
import { loadMyUserId } from "@/api/members";
import { Empty } from "@/components/project/items";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { timeAgo } from "@/lib/format";
import { useReloading } from "@/lib/reloading";
import { useProjectEvents } from "@/live/context";

type Params = { workspaceId: string; projectId: string };

function whoOf(author: Record<string, unknown>): string {
  if (author.kind === "human") return "human";
  return typeof author.role === "string" && author.role ? author.role : "agent";
}

function Heading({ count }: { count: number }) {
  return (
    <div className="flex items-baseline gap-2.5">
      <h2 id="decisions-title" className="text-[15px] font-semibold tracking-tight">
        Decisions
      </h2>
      <span className="text-sm text-muted-foreground tabular-nums">{count}</span>
    </div>
  );
}

function Entry({
  decision,
  replaced,
  showReplaces,
}: {
  decision: DecisionBody;
  replaced: Map<string, DecisionBody[]>;
  showReplaces: boolean;
}) {
  const earlier = replaced.get(decision.id) ?? [];
  return (
    <li className="space-y-1.5 rounded-lg border px-3.5 py-3 text-sm">
      <p className="text-[13px] text-muted-foreground">{decision.question}</p>
      <p className="font-medium">{decision.decision}</p>
      {decision.why && <p className="whitespace-pre-line text-[13px] text-foreground/80">{decision.why}</p>}
      <p className="text-[12.5px] text-muted-foreground">
        {whoOf(decision.author)} · {timeAgo(decision.created_at)}
        {showReplaces && decision.replaces_decision_id && " · replaces an earlier decision"}
      </p>
      {earlier.length > 0 && (
        <ul aria-label="Replaced decisions" className="ml-3 space-y-2 border-l pl-3">
          {earlier.map((one) => (
            <Entry key={one.id} decision={one} replaced={replaced} showReplaces={showReplaces} />
          ))}
        </ul>
      )}
    </li>
  );
}

/** A flat list, newest first, with a note on a decision that replaces another. */
function FlatList({ decisions }: { decisions: DecisionBody[] }) {
  const none = new Map<string, DecisionBody[]>();
  return (
    <ul className="space-y-2">
      {decisions.map((decision) => (
        <Entry key={decision.id} decision={decision} replaced={none} showReplaces />
      ))}
    </ul>
  );
}

/** Decisions nobody has replaced, each with the ones it replaced nested under it. */
function NestedList({ decisions }: { decisions: DecisionBody[] }) {
  const ids = new Set(decisions.map((decision) => decision.id));
  const replaced = new Map<string, DecisionBody[]>();
  const top: DecisionBody[] = [];
  for (const decision of decisions) {
    const by = decision.replaced_by_decision_id;
    if (by && ids.has(by)) replaced.set(by, [...(replaced.get(by) ?? []), decision]);
    else top.push(decision);
  }
  return (
    <ul className="space-y-2">
      {top.map((decision) => (
        <Entry key={decision.id} decision={decision} replaced={replaced} showReplaces={false} />
      ))}
    </ul>
  );
}

/** The decisions linked to a feature; a replaced decision sits under the one replacing it. */
export function FeatureDecisions({ workspaceId, projectId, partId, featureId }: Params & { partId: string; featureId: string }) {
  const load = useCallback(
    () => loadFeatureDecisions(workspaceId, projectId, partId, featureId),
    [workspaceId, projectId, partId, featureId],
  );
  const { value, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  return (
    <section id="decisions" aria-labelledby="decisions-title" className="scroll-mt-6 space-y-3">
      <Heading count={value?.length ?? 0} />
      {value && (value.length === 0 ? <Empty>No decisions are linked to this feature</Empty> : <NestedList decisions={value} />)}
    </section>
  );
}

function messageOf(failure: unknown): string {
  if (failure instanceof ApiError && failure.code === "not_track_owner") return "Only the track's owner can record a decision";
  return "The decision could not be recorded. Try again.";
}

function AddDecision({ workspaceId, projectId, trackId, onAdded }: Params & { trackId: string; onAdded: () => void }) {
  const loadFeatures = useCallback(() => loadTrackFeatures(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const features = useReloading(loadFeatures);
  const [question, setQuestion] = useState("");
  const [decision, setDecision] = useState("");
  const [why, setWhy] = useState("");
  const [feature, setFeature] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ready = question.trim() !== "" && decision.trim() !== "" && why.trim() !== "";

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      await recordDecision(workspaceId, projectId, trackId, {
        question: question.trim(),
        decision: decision.trim(),
        why: why.trim(),
        feature_id: feature || null,
      });
      setQuestion("");
      setDecision("");
      setWhy("");
      setFeature("");
      onAdded();
    } catch (failure) {
      setError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      aria-label="Add decision"
      className="space-y-2 rounded-lg border p-3.5"
      onSubmit={(event) => {
        event.preventDefault();
        if (ready && !busy) void submit();
      }}
    >
      <Input aria-label="Question" placeholder="Question" value={question} onChange={(event) => setQuestion(event.target.value)} />
      <Input aria-label="Decision" placeholder="Decision" value={decision} onChange={(event) => setDecision(event.target.value)} />
      <Input aria-label="Why" placeholder="Why" value={why} onChange={(event) => setWhy(event.target.value)} />
      <div className="flex gap-2">
        <select
          aria-label="Feature"
          value={feature}
          className="h-8 max-w-xs rounded-lg border bg-background px-2 text-sm"
          onChange={(event) => setFeature(event.target.value)}
        >
          <option value="">No feature</option>
          {(features.value ?? []).map((one) => (
            <option key={one.id} value={one.id}>
              {one.name}
            </option>
          ))}
        </select>
        <Button type="submit" size="sm" disabled={!ready || busy}>
          Add decision
        </Button>
      </div>
      {error && (
        <Alert variant="destructive">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}
    </form>
  );
}

/** The decisions recorded on a track, newest first, and for its owner a form to add one. */
export function TrackDecisions({ workspaceId, projectId, trackId, ownerUserId }: Params & { trackId: string; ownerUserId: string | null }) {
  const load = useCallback(() => loadTrackDecisions(workspaceId, projectId, trackId), [workspaceId, projectId, trackId]);
  const { value, reload } = useReloading(load);
  useProjectEvents(projectId, reload);
  const loadMe = useCallback(() => loadMyUserId(workspaceId), [workspaceId]);
  const me = useReloading(loadMe);
  const mine = ownerUserId !== null && me.value === ownerUserId;
  return (
    <section id="decisions" aria-labelledby="decisions-title" className="scroll-mt-6 space-y-3">
      <Heading count={value?.length ?? 0} />
      {mine && <AddDecision workspaceId={workspaceId} projectId={projectId} trackId={trackId} onAdded={reload} />}
      {value && (value.length === 0 ? <Empty>No decisions yet</Empty> : <FlatList decisions={value} />)}
    </section>
  );
}
