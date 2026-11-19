import { useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Box, ChevronRight, Plus } from "lucide-react";
import { ApiError } from "@/api/client";
import { recordDecision } from "@/api/decisions";
import type { DecisionBody, FeatureBody, PartBody, TrackSummary } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { StateIcon } from "@/components/shell/state-icon";
import { cn } from "@/lib/utils";
import { authorOf, type People } from "./people";
import { Drawer, DialogError } from "@/components/shell/dialog";
import { Field, Input, Textarea } from "@/components/shell/field";
import { Select } from "@/components/shell/select";
import { EmptyLine, buttonClass, chipClass, shortDate } from "./ui";

// A decision log, as the feature's and the track's Decisions show it: the
// current decisions newest first, each with why, who, the question it answers
// and where it came from (a feature's: the track that recorded it) or what it
// is about (a track's: its feature, part or the project); a decision another
// replaced is folded under the one that replaced it. A track's owner records
// decisions on it with the form above the log.

type Params = { workspaceId: string; projectId: string };

/** The decisions nobody in `all` replaced, newest first, each with those it replaced (and they theirs) folded under it. */
export function foldReplaced(all: DecisionBody[], shown: DecisionBody[] = all) {
  const ids = new Set(all.map((decision) => decision.id));
  const under = new Map<string, DecisionBody[]>();
  const top: DecisionBody[] = [];
  for (const decision of all) {
    const by = decision.replaced_by_decision_id;
    if (by && ids.has(by)) under.set(by, [...(under.get(by) ?? []), decision]);
    else if (shown.includes(decision)) top.push(decision);
  }
  return { top, under, folded: all.length - top.length };
}

export type LogContext = {
  params: Params;
  people: People | null;
  /** The project's tracks: the feature's log says which track a decision came from. */
  tracks?: TrackSummary[];
  /** "from": the track a decision was recorded on; "about": the feature, part or project it is about. */
  show: "from" | "about";
};

const metaLink = "inline-flex items-center gap-1 font-medium text-ink-2 hover:text-ink hover:underline underline-offset-2 [&>svg]:size-3";

function Where({ decision, context }: { decision: DecisionBody; context: LogContext }) {
  const { params, tracks, show } = context;
  if (show === "from") {
    const track = tracks?.find((one) => one.track.id === decision.track_id);
    if (!track) return null;
    return (
      <>
        {" · "}
        <span className="whitespace-nowrap">
          {"from "}
          <Link to="/w/$workspaceId/projects/$projectId/tracks/$trackId" params={{ ...params, trackId: track.track.id }} className={metaLink}>
            <StateIcon state={track.track.state} />
            {track.track.name}
          </Link>
        </span>
      </>
    );
  }
  if (decision.about === "feature" && decision.feature_id) {
    return (
      <>
        {" · "}
        <span className="whitespace-nowrap">
          {"about "}
          <span className={metaLink}>
            <Box aria-hidden strokeWidth={2.25} />
            {decision.feature_name ?? "a feature"}
          </span>
        </span>
      </>
    );
  }
  if (decision.about === "part" && decision.part_id) {
    return (
      <>
        {" · "}
        <span className="whitespace-nowrap">
          {"about "}
          <Link to="/w/$workspaceId/projects/$projectId/parts/$partId" params={{ ...params, partId: decision.part_id }} className={metaLink}>
            {decision.part_name ?? "a part"}
          </Link>
        </span>
      </>
    );
  }
  if (decision.about === "project") return <> · about the project</>;
  return null;
}

function Entry({
  decision,
  under,
  context,
  replaced = false,
}: {
  decision: DecisionBody;
  under: Map<string, DecisionBody[]>;
  context: LogContext;
  replaced?: boolean;
}) {
  const author = authorOf(context.people, decision.author);
  const earlier = under.get(decision.id) ?? [];
  const [open, setOpen] = useState(false);
  return (
    <li className={cn("grid grid-cols-[var(--avatar)_minmax(0,1fr)_auto] gap-x-3", replaced ? "pt-3" : "border-b border-line py-3")}>
      <span className="pt-0.5">
        <Avatar name={author.name} agent={author.agent} tintKey={author.key} />
      </span>
      <div className="min-w-0">
        <p className={cn("font-medium", replaced ? "text-ink-3 line-through decoration-ink-3" : "text-ink")}>{decision.decision}</p>
        {decision.why && <p className="mt-0.5 whitespace-pre-line text-ink-2">Why: {decision.why}</p>}
        <p className="mt-1 text-meta text-ink-3">
          {author.name} · Answers &ldquo;{decision.question}&rdquo;
          <Where decision={decision} context={context} />
        </p>
        {earlier.length > 0 && (
          <div className="mt-1.5">
            <button
              type="button"
              aria-expanded={open}
              onClick={() => setOpen(!open)}
              className="inline-flex max-w-full items-center gap-1 text-meta text-ink-2 hover:text-ink"
            >
              <ChevronRight aria-hidden className={cn("size-3 shrink-0 transition-transform duration-(--t-fast)", open && "rotate-90")} />
              Replaces
              {!open && <span className="truncate text-ink-3 line-through">{earlier.map((one) => one.decision).join(", ")}</span>}
            </button>
            {open && (
              <ul aria-label="Replaced decisions" className="ml-1.5 border-l border-line pl-3">
                {earlier.map((one) => (
                  <Entry key={one.id} decision={one} under={under} context={context} replaced />
                ))}
              </ul>
            )}
          </div>
        )}
      </div>
      <time dateTime={decision.created_at} title={new Date(decision.created_at).toLocaleString()} className="pt-0.5 text-meta whitespace-nowrap text-ink-3">
        {shortDate(decision.created_at)}
      </time>
    </li>
  );
}

/** The log: the decisions nobody replaced, newest first, with the replaced ones folded under them. */
export function DecisionLog({
  decisions,
  earlier = [],
  context,
  empty,
  limit,
}: {
  decisions: DecisionBody[];
  /** Decisions of other levels that came with the list because one of `decisions` replaced them. */
  earlier?: DecisionBody[];
  context: LogContext;
  empty: ReactNode;
  /** Show only the newest this many. */
  limit?: number;
}) {
  const { top, under } = foldReplaced([...decisions, ...earlier], decisions);
  if (top.length === 0) return <EmptyLine>{empty}</EmptyLine>;
  return (
    <ul aria-label="Decisions" className="border-t border-line">
      {top.slice(0, limit).map((decision) => (
        <Entry key={decision.id} decision={decision} under={under} context={context} />
      ))}
    </ul>
  );
}

function recordError(failure: unknown): string {
  if (failure instanceof ApiError && failure.code === "not_track_owner") return "Only the track's owner can record a decision.";
  return "The decision could not be recorded. Try again.";
}

/**
 * The drawer that records a decision on a track: what was decided, the
 * question it answers, why, and what it is about (the track, one of its
 * features, its part, or the whole project). All but About are required; one
 * left empty is said under it when the drawer is submitted.
 */
export function RecordDecision({
  params,
  trackId,
  trackName,
  trackState = "planning",
  by,
  features,
  part,
  onDone,
  onCancel,
}: {
  params: Params;
  trackId: string;
  trackName?: string;
  trackState?: string;
  /** Who records it: the track's owner. */
  by?: string;
  features: FeatureBody[];
  part: PartBody | undefined;
  onDone: () => void;
  onCancel: () => void;
}) {
  const [question, setQuestion] = useState("");
  const [decision, setDecision] = useState("");
  const [why, setWhy] = useState("");
  const [about, setAbout] = useState("track");
  const [busy, setBusy] = useState(false);
  const [tried, setTried] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const missing = (value: string) => tried && value.trim() === "";

  async function submit() {
    setTried(true);
    if (question.trim() === "" || decision.trim() === "" || why.trim() === "") {
      requestAnimationFrame(() => document.querySelector<HTMLElement>('[role="dialog"] [aria-invalid="true"]')?.focus());
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await recordDecision(params.workspaceId, params.projectId, trackId, {
        question: question.trim(),
        decision: decision.trim(),
        why: why.trim(),
        feature_id: about.startsWith("feature:") ? about.slice("feature:".length) : null,
        ...(about === "part" && part ? { part_id: part.id } : {}),
        ...(about === "project" ? { about_project: true } : {}),
      });
      onDone();
    } catch (failure) {
      setError(recordError(failure));
    } finally {
      setBusy(false);
    }
  }

  const options = [
    { value: "track", label: "This track", hint: trackName },
    ...features.map((feature) => ({ value: `feature:${feature.id}`, label: feature.name, hint: "feature", icon: <Box aria-hidden strokeWidth={2.25} className="text-ink-2" /> })),
    ...(part ? [{ value: "part", label: part.name, hint: "part" }] : []),
    { value: "project", label: "The whole project" },
  ];

  return (
    <Drawer
      title="Record a decision"
      description="A decision made outside a thread. It shows here, on what it is about and in the decision log."
      onClose={onCancel}
      onSubmit={() => {
        if (!busy) void submit();
      }}
      footerStart={error && <DialogError>{error}</DialogError>}
      footer={
        <>
          <button type="button" className={buttonClass()} onClick={onCancel}>
            Cancel
          </button>
          <button type="submit" className={buttonClass({ variant: "primary" })} disabled={busy}>
            Record decision
          </button>
        </>
      }
    >
      <Field label="What was decided" error={missing(decision) ? "Say what was decided" : undefined}>
        {(control) => <Input {...control} placeholder="One sentence, as it now stands" value={decision} onChange={(event) => setDecision(event.target.value)} />}
      </Field>
      <Field label="Question it answers" error={missing(question) ? "Say which question it answers" : undefined}>
        {(control) => <Input {...control} placeholder="The question that needed deciding" value={question} onChange={(event) => setQuestion(event.target.value)} />}
      </Field>
      <Field label="Why" error={missing(why) ? "Say why" : undefined}>
        {(control) => <Textarea {...control} placeholder="What made this the right call" value={why} onChange={(event) => setWhy(event.target.value)} />}
      </Field>
      <Field label="About" hint="The decision shows on what it is about too.">
        {(control) => <Select {...control} value={about} onChange={setAbout} options={options} />}
      </Field>
      {trackName && (
        <Field label="Recorded on">
          {(control) => (
            <span id={control.id} className="flex min-h-(--control-h-lg) items-center gap-2">
              <span className={chipClass}>
                <StateIcon state={trackState} />
                <span className="truncate">{trackName}</span>
              </span>
              {by && <span className="text-ink-3">by {by}</span>}
            </span>
          )}
        </Field>
      )}
    </Drawer>
  );
}

/** The "Record decision" button: the owner opens the form with it; anyone else sees it off, with why. */
export function RecordButton({ allowed, owner, onClick }: { allowed: boolean; owner: string; onClick: () => void }) {
  return (
    <button
      type="button"
      className={buttonClass()}
      disabled={!allowed}
      title={allowed ? undefined : `Only ${owner} records decisions on this track`}
      onClick={onClick}
    >
      <Plus aria-hidden strokeWidth={2.25} />
      Record decision
    </button>
  );
}
