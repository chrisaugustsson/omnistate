import { useMemo, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { Box, ChevronRight } from "lucide-react";
import type { DecisionBody, PartBody, TrackSummary } from "@/api/generated";
import { Avatar } from "@/components/shell/avatar";
import { PartIcon, ProjectMark } from "@/components/shell/part-icon";
import { StateIcon, stateLabel } from "@/components/shell/state-icon";
import { partTone } from "@/lib/identity";
import { cn } from "@/lib/utils";
import { authorOf, isReplaced, type DecisionGroup, type Subject } from "./project-data";
import { Band, EmptyRow, RowLead, Sep, shortDate } from "./rows";

// Decisions as rows: on an Overview the latest few (what was decided, who,
// what it is about, which track it came from), and on a Decisions tab the log
// of a level grouped by what each decision is about, with the decisions each
// one replaced folded under it.

type Params = { workspaceId: string; projectId: string };

/** What the rows need to name people, parts and tracks. */
export type DecisionContext = {
  params: Params;
  names: ReadonlyMap<string, string> | null;
  tracks: readonly TrackSummary[];
  parts: readonly PartBody[] | null;
};

const fromClass = "inline-flex items-center gap-1 align-top font-medium whitespace-nowrap text-ink-2 hover:text-ink";

/** The track a decision came from: its state icon and name, a link to it. */
function FromTrack({ trackId, context }: { trackId: string; context: DecisionContext }) {
  const summary = context.tracks.find((candidate) => candidate.track.id === trackId);
  if (!summary) return <span className="text-ink-2">a track</span>;
  return (
    <Link
      to="/w/$workspaceId/projects/$projectId/tracks/$trackId"
      params={{ ...context.params, trackId }}
      className={fromClass}
      title={`Track, ${stateLabel(summary.track.state).toLowerCase()}`}
    >
      <StateIcon state={summary.track.state} className="size-3" />
      {summary.track.name}
    </Link>
  );
}

/** What a decision is about, as a link: the project, a part, or a part's feature. */
function About({ subject, context, withPart }: { subject: Subject; context: DecisionContext; withPart: boolean }) {
  const { params, parts } = context;
  if (subject.kind === "project") {
    return (
      <Link to="/w/$workspaceId/projects/$projectId" params={params} className={fromClass}>
        <ProjectMark name={subject.name} parts={parts?.map((part) => ({ kind: part.kind, tone: partTone(parts, part.id) }))} className="size-3 rounded-[3px]" />
        {subject.name}
      </Link>
    );
  }
  const icon = <PartIcon name={subject.part.name} kind={subject.part.kind} tone={partTone(parts, subject.part.id)} size="sm" />;
  if (subject.kind === "part") {
    return (
      <Link to="/w/$workspaceId/projects/$projectId/parts/$partId" params={{ ...params, partId: subject.id }} className={fromClass}>
        {icon}
        {subject.name}
      </Link>
    );
  }
  return (
    <Link
      to="/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId"
      params={{ ...params, partId: subject.part.id, featureId: subject.id }}
      className={fromClass}
    >
      {withPart ? icon : <Box aria-hidden className="size-3 text-ink-2" strokeWidth={1.75} />}
      {withPart ? `${subject.part.name} › ${subject.name}` : subject.name}
    </Link>
  );
}

function Author({ decision, context }: { decision: DecisionBody; context: DecisionContext }) {
  const who = authorOf(decision.author, context.names);
  return <Avatar name={who.name} agent={who.agent} tintKey={who.key} title={who.name} />;
}

/** The decisions `decision` replaced, the latest first, following the chain back. */
function replacedChain(decision: DecisionBody, byId: ReadonlyMap<string, DecisionBody>): DecisionBody[] {
  const chain: DecisionBody[] = [];
  let next = decision.replaces_decision_id ? byId.get(decision.replaces_decision_id) : undefined;
  while (next && !chain.includes(next)) {
    chain.push(next);
    next = next.replaces_decision_id ? byId.get(next.replaces_decision_id) : undefined;
  }
  return chain;
}

/** A decision on an Overview: what was decided, then who, what it is about, its track and how many it replaces; its date. */
export function RecentDecisionRow({
  decision,
  subject,
  context,
  showAbout,
}: {
  decision: DecisionBody;
  subject: Subject;
  context: DecisionContext;
  /** Name what it is about (on the project's Overview, where decisions are of every level). */
  showAbout: boolean;
}) {
  const who = authorOf(decision.author, context.names);
  return (
    <li className="flex min-h-(--row-h) items-start gap-3 border-t border-line py-2.5">
      <RowLead two>
        <Author decision={decision} context={context} />
      </RowLead>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-medium">{decision.decision}</span>
        <span className="text-meta leading-5 text-ink-3">
          {who.name}
          {(showAbout || subject.kind === "feature") && (
            <>
              <Sep />
              about <About subject={subject} context={context} withPart={showAbout} />
            </>
          )}
          <Sep />
          from <FromTrack trackId={decision.track_id} context={context} />
          {decision.replaces_decision_id && (
            <>
              <Sep />
              replaces an earlier decision
            </>
          )}
        </span>
      </span>
      <time dateTime={decision.created_at} className="pt-px text-meta whitespace-nowrap text-ink-3">
        {shortDate(decision.created_at)}
      </time>
    </li>
  );
}

/** A decision in the log: what was decided, why, who, the question it answers and its track; what it replaced folded under it. */
function LogRow({
  decision,
  context,
  byId,
  replaced,
}: {
  decision: DecisionBody;
  context: DecisionContext;
  byId: ReadonlyMap<string, DecisionBody>;
  /** A later decision replaced this one (shown with "Show replaced" on, or its successor is about another level). */
  replaced?: boolean;
}) {
  const who = authorOf(decision.author, context.names);
  const chain = replacedChain(decision, byId);
  return (
    <li className="flex items-start gap-3 border-t border-line px-3 py-2.5 first:border-t-0">
      <RowLead two>
        <Author decision={decision} context={context} />
      </RowLead>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <p className={cn("font-medium", replaced && "text-ink-2")}>
          {decision.decision}
          {replaced && <span className="ml-2 align-[1px] text-meta font-medium text-ink-3">replaced</span>}
        </p>
        {decision.why && <p className="whitespace-pre-line text-ink-2">Why: {decision.why}</p>}
        <p className="text-meta leading-5 text-ink-3">
          {who.name}
          <Sep />
          Answers “{decision.question}”
          <Sep />
          from <FromTrack trackId={decision.track_id} context={context} />
        </p>
        {chain.length > 0 && <Replaces chain={chain} context={context} />}
      </div>
      <time dateTime={decision.created_at} className="pt-px text-meta whitespace-nowrap text-ink-3">
        {shortDate(decision.created_at)}
      </time>
    </li>
  );
}

/** The decisions a decision replaced, folded: "Replaces <the last one>", opening to each with its why and who. */
function Replaces({ chain, context }: { chain: DecisionBody[]; context: DecisionContext }) {
  return (
    <details className="group/replaces mt-1 text-meta">
      <summary className="inline-flex cursor-pointer list-none items-center gap-1.5 rounded-badge text-ink-2 hover:text-ink focus-visible:shadow-focus focus-visible:outline-none [&::-webkit-details-marker]:hidden">
        <ChevronRight aria-hidden className="size-3 transition-transform duration-(--t-fast) group-open/replaces:rotate-90" />
        Replaces
        <s className="text-ink-3">{chain[0].decision}</s>
        {chain.length > 1 && <span className="text-ink-3">and {chain.length - 1} before it</span>}
      </summary>
      <ol aria-label="Replaced decisions" className="mt-1.5 ml-[5px] space-y-2 border-l border-line pl-3">
        {chain.map((old) => (
          <li key={old.id}>
            <p className="font-medium text-ink-2 line-through decoration-ink-3">{old.decision}</p>
            {old.why && <p className="text-ink-3">Why: {old.why}</p>}
            <p className="text-ink-3">
              {authorOf(old.author, context.names).name}
              <Sep />
              {shortDate(old.created_at)}
              <Sep />
              from <FromTrack trackId={old.track_id} context={context} />
            </p>
          </li>
        ))}
      </ol>
    </details>
  );
}

/** The kind of a subject, quiet, after its name on its band. */
function kindOf(subject: Subject): string {
  if (subject.kind === "project") return "the project";
  if (subject.kind === "part") return subject.part.kind;
  return `feature of ${subject.part.name}`;
}

function SubjectIcon({ subject, parts }: { subject: Subject; parts: readonly PartBody[] | null }) {
  if (subject.kind === "project") {
    return <ProjectMark name={subject.name} parts={parts?.map((part) => ({ kind: part.kind, tone: partTone(parts, part.id) }))} className="size-(--icon-sm) rounded-[3px] grid-cols-[repeat(2,3px)] grid-rows-[repeat(2,3px)]" />;
  }
  if (subject.kind === "part") return <PartIcon name={subject.name} kind={subject.part.kind} tone={partTone(parts, subject.id)} size="sm" />;
  return <Box aria-hidden className="size-(--icon-sm) text-ink-2" strokeWidth={1.75} />;
}

function subjectLink(subject: Subject, params: Params) {
  if (subject.kind === "project") return { to: "/w/$workspaceId/projects/$projectId", params } as const;
  if (subject.kind === "part") return { to: "/w/$workspaceId/projects/$projectId/parts/$partId", params: { ...params, partId: subject.id } } as const;
  return {
    to: "/w/$workspaceId/projects/$projectId/parts/$partId/features/$featureId",
    params: { ...params, partId: subject.part.id, featureId: subject.id },
  } as const;
}

/** A choice of the segmented filter: its label and which groups it keeps. */
type Choice = { key: string; label: ReactNode; keeps: (subject: Subject) => boolean };

/**
 * The decision log of a level: a filter over what the decisions are about,
 * "Show replaced", and the decisions grouped by subject, newest first. A
 * decision a later one replaced is folded under its successor; with "Show
 * replaced" on it also has its own row. One whose successor is not in the log
 * (it is about another level) keeps its row.
 */
export function DecisionLog({
  groups,
  context,
  level,
  emptyText,
}: {
  groups: readonly DecisionGroup[];
  context: DecisionContext;
  /** The project's log filters by kind of subject; a part's by subject. */
  level: "project" | "part";
  emptyText: ReactNode;
}) {
  const [choice, setChoice] = useState("all");
  const [showReplaced, setShowReplaced] = useState(false);
  const { byId, shownIds } = useMemo(() => {
    const byId = new Map<string, DecisionBody>();
    for (const group of groups) for (const decision of [...group.earlier, ...group.decisions]) byId.set(decision.id, decision);
    const shownIds = new Set(groups.flatMap((group) => group.decisions.map((decision) => decision.id)));
    return { byId, shownIds };
  }, [groups]);
  /** Folded under a successor that the log shows. */
  const folded = (decision: DecisionBody) => isReplaced(decision) && shownIds.has(decision.replaced_by_decision_id ?? "");
  const current = (group: DecisionGroup) => group.decisions.filter((decision) => !folded(decision));

  const choices: Choice[] =
    level === "project"
      ? [
          { key: "all", label: "All", keeps: () => true },
          { key: "project", label: "Project", keeps: (subject) => subject.kind === "project" },
          { key: "part", label: "Parts", keeps: (subject) => subject.kind === "part" },
          { key: "feature", label: "Features", keeps: (subject) => subject.kind === "feature" },
        ]
      : [
          { key: "all", label: "All", keeps: () => true },
          ...groups.map((group) => ({ key: group.subject.id, label: group.subject.name, keeps: (subject: Subject) => subject.id === group.subject.id })),
        ];
  const countOf = (keeps: Choice["keeps"]) => groups.filter((group) => keeps(group.subject)).reduce((sum, group) => sum + current(group).length, 0);
  const chosen = choices.find((candidate) => candidate.key === choice) ?? choices[0];
  const shown = groups
    .filter((group) => chosen.keeps(group.subject))
    .map((group) => ({ group, rows: showReplaced ? group.decisions : current(group) }))
    .filter(({ rows }) => rows.length > 0);
  const total = countOf(() => true);

  return (
    <div className="mt-6">
      <div className="flex flex-wrap items-center gap-3">
        <div role="radiogroup" aria-label="Decisions about" className="inline-flex flex-wrap gap-0.5 rounded-control bg-hover p-0.5">
          {choices.map((candidate) => (
            <button
              key={candidate.key}
              type="button"
              role="radio"
              aria-checked={candidate.key === chosen.key}
              onClick={() => setChoice(candidate.key)}
              className={cn(
                "inline-flex h-6 items-center gap-1.5 rounded-badge px-2.5 font-medium whitespace-nowrap text-ink-2 transition-colors duration-(--t-fast) hover:text-ink",
                "focus-visible:shadow-focus focus-visible:outline-none",
                "aria-checked:bg-sheet aria-checked:text-ink aria-checked:shadow-[0_0_0_1px_var(--line),var(--shadow-control-value)]",
              )}
            >
              {candidate.label}
              <span className="text-ink-3 tabular-nums">{countOf(candidate.keeps)}</span>
            </button>
          ))}
        </div>
        <label className="ml-auto inline-flex cursor-pointer items-center gap-2 text-ink-2">
          <span className="relative inline-flex h-4 w-7 shrink-0">
            <input
              type="checkbox"
              checked={showReplaced}
              onChange={(event) => setShowReplaced(event.target.checked)}
              className="peer absolute inset-0 m-0 cursor-pointer opacity-0"
            />
            <span
              aria-hidden
              className="pointer-events-none absolute inset-0 rounded-full bg-line-strong transition-colors duration-(--t-fast) peer-checked:bg-ink peer-focus-visible:shadow-focus after:absolute after:top-0.5 after:left-0.5 after:size-3 after:rounded-full after:bg-sheet after:shadow-control after:transition-transform after:duration-(--t-pop) peer-checked:after:translate-x-3"
            />
          </span>
          Show replaced
        </label>
      </div>
      {total === 0 && !showReplaced ? (
        <ul className="mt-3 border-y border-line">
          <EmptyRow>{emptyText}</EmptyRow>
        </ul>
      ) : (
        <div className="-mx-3 mt-3">
          {shown.map(({ group, rows }) => (
            <section key={group.subject.id} aria-label={group.subject.name} className="mb-3">
              <Band icon={<SubjectIcon subject={group.subject} parts={context.parts} />} count={rows.length}>
                <Link {...subjectLink(group.subject, context.params)} className="-mx-1.5 inline-flex h-6 items-center rounded-badge px-1.5 hover:bg-hover">
                  {group.subject.name}
                </Link>
                <span className="text-meta font-normal text-ink-3">{kindOf(group.subject)}</span>
              </Band>
              <ul className="border-b border-line">
                {rows.map((decision) => (
                  <LogRow key={decision.id} decision={decision} context={context} byId={byId} replaced={isReplaced(decision)} />
                ))}
              </ul>
            </section>
          ))}
          {shown.length === 0 && (
            <ul className="mx-3 border-y border-line">
              <EmptyRow>No decisions about this yet.</EmptyRow>
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
