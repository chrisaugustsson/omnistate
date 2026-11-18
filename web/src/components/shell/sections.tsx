import { useState, type ComponentType } from "react";
import { Link } from "@tanstack/react-router";
import {
  BookOpen,
  Box,
  Boxes,
  ChevronsRight,
  ChevronUp,
  Columns3,
  FileText,
  Inbox,
  Layers,
  MessagesSquare,
  PanelLeftClose,
  PenTool,
  Scale,
  ScrollText,
  UserRound,
  Waypoints,
  Workflow,
  Wrench,
} from "lucide-react";
import type { HomeResponse, PartBody, ProjectBody } from "@/api/generated";
import { partDocs } from "@/components/project/docs-pane";
import { isProjectLevel } from "@/components/project/items";
import { currentDecisions, useDecisionGroups, useProjectMap } from "@/components/project/project-data";
import { useFeatures } from "@/lib/use-features";
import { cn } from "@/lib/utils";
import { BottomSheet } from "./sheet";
import { SECTION_LABELS, sectionLink, sectionsOf, type Scope, type Section } from "./scope";
import { countsKey, useReportSections, useReportedSections, type SectionCount } from "./shell-context";
import { inFlight } from "./state-icon";

// The sections of the path's last step (navigation F): the sidebar on a
// desktop, the section button and its sheet on the phone. Each section has an
// icon, a label and a count: a pale count is a size, a dark one says that
// something waits on the reader there. The workspace's, a project's and a
// part's counts are read here; a feature's and a track's come from their
// layouts (useReportSections).

const ICONS: Record<Section, ComponentType<{ className?: string; strokeWidth?: number; "aria-hidden"?: boolean }>> = {
  inbox: Inbox,
  "my-tracks": UserRound,
  projects: Boxes,
  overview: BookOpen,
  parts: Layers,
  features: Box,
  tracks: Waypoints,
  docs: FileText,
  decisions: Scale,
  design: PenTool,
  spec: ScrollText,
  behaviour: Workflow,
  how: Wrench,
  board: Columns3,
  threads: MessagesSquare,
};

/** The key of the open level's counts. */
export function scopeKey(scope: Scope): string {
  const id = { workspace: scope.workspaceId, project: scope.projectId, part: scope.partId, feature: scope.featureId, track: scope.trackId }[scope.level];
  return countsKey(scope.level, id);
}

/** The sections the open level shows, with their counts. */
export function useSections(scope: Scope): { section: Section; count: SectionCount }[] {
  const counts = useReportedSections(scopeKey(scope));
  return sectionsOf(scope.level)
    .map((section) => ({ section, count: counts[section] ?? {} }))
    .filter(({ count }) => !count.hidden);
}

/** How many things wait on the reader across the sections. */
export function waitingIn(sections: readonly { count: SectionCount }[]): number {
  return sections.reduce((sum, { count }) => sum + (count.waiting ?? 0), 0);
}

const size = (count: number | undefined): SectionCount => (count ? { count } : {});

/** How many of `home`'s tracks are in flight (planning or building), on the part `partId` when given. */
function tracksInFlight(home: HomeResponse | undefined, partId?: string): number | undefined {
  if (!home) return undefined;
  return (home.tracks ?? []).filter(({ track }) => inFlight(track.state) && (!partId || track.part_id === partId)).length;
}

/** The workspace's counts: what waits on the reader, their tracks in flight, the projects. */
function WorkspaceCounts({
  scope,
  projects,
  homes,
  me,
  waiting,
}: {
  scope: Scope;
  projects: readonly ProjectBody[] | null;
  homes: ReadonlyMap<string, HomeResponse> | null;
  me: string | null;
  waiting: number | null;
}) {
  let mine: number | undefined;
  if (homes && me) {
    mine = 0;
    for (const home of homes.values()) mine += (home.tracks ?? []).filter(({ track }) => track.owner_user_id === me && inFlight(track.state)).length;
  }
  useReportSections(scopeKey(scope), {
    inbox: waiting ? { waiting, note: `${waiting} for you` } : {},
    "my-tracks": mine ? { count: mine, note: `${mine} in flight` } : {},
    projects: size(projects?.length),
  });
  return null;
}

/** How many decisions the project (no `partId`) or a part lists: those nobody replaced. */
function useDecisionCount(workspaceId: string, projectId: string, projectName: string | undefined, partId?: string) {
  const { map } = useProjectMap(workspaceId, projectId);
  const { groups } = useDecisionGroups(workspaceId, projectId, projectName, map, partId);
  return groups ? currentDecisions(groups).length : undefined;
}

function ProjectCounts({
  scope,
  project,
  parts,
  home,
}: {
  scope: Scope;
  project: ProjectBody | undefined;
  parts: readonly PartBody[] | null;
  home: HomeResponse | undefined;
}) {
  const decisions = useDecisionCount(scope.workspaceId, scope.projectId!, project?.name);
  useReportSections(scopeKey(scope), {
    parts: size(parts?.length),
    tracks: size(tracksInFlight(home)),
    docs: size(home?.docs.filter(isProjectLevel).length),
    decisions: size(decisions),
  });
  return null;
}

function PartCounts({ scope, project, home }: { scope: Scope; project: ProjectBody | undefined; home: HomeResponse | undefined }) {
  const projectId = scope.projectId!;
  const partId = scope.partId!;
  const { features } = useFeatures(scope.workspaceId, projectId, partId);
  const decisions = useDecisionCount(scope.workspaceId, projectId, project?.name, partId);
  useReportSections(scopeKey(scope), {
    features: size(features?.length),
    tracks: size(tracksInFlight(home, partId)),
    // As the part's Docs section lists them: on the part and on its features.
    docs: size(home && features ? partDocs(home.docs, partId, new Set(features.map((feature) => feature.id))).length : undefined),
    decisions: size(decisions),
  });
  return null;
}

/** Reads the counts of the open level when the shell knows them itself (the workspace, a project, a part). */
export function LevelCounts(props: {
  scope: Scope;
  projects: readonly ProjectBody[] | null;
  parts: readonly PartBody[] | null;
  homes: ReadonlyMap<string, HomeResponse> | null;
  me: string | null;
  waiting: number | null;
}) {
  const { scope, projects, parts, homes } = props;
  const project = projects?.find((candidate) => candidate.id === scope.projectId);
  const home = scope.projectId ? homes?.get(scope.projectId) : undefined;
  if (scope.level === "workspace") return <WorkspaceCounts {...props} />;
  if (scope.level === "project" && scope.projectId) return <ProjectCounts key={scope.projectId} scope={scope} project={project} parts={parts} home={home} />;
  if (scope.level === "part" && scope.projectId && scope.partId) return <PartCounts key={scope.partId} scope={scope} project={project} home={home} />;
  return null;
}

/** A dark count: something waits on the reader. */
export function WaitingCount({ count, className }: { count: number; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-ink px-[5px] text-[10.5px] leading-none font-semibold text-on-fill tabular-nums",
        className,
      )}
    >
      {count}
    </span>
  );
}

function countLabel(count: SectionCount): string {
  if (count.waiting) return `, ${count.waiting} waiting on you`;
  return count.count !== undefined ? `, ${count.count}` : "";
}

const FOLD_KEY = "omni.sidebar.folded";

function readFolded(): boolean {
  try {
    return localStorage.getItem(FOLD_KEY) === "1";
  } catch {
    return false;
  }
}

/**
 * The sidebar's fold: only what the reader chose with the button at its foot,
 * kept in this browser. No page folds it on its own.
 */
function useFold() {
  const [folded, setFolded] = useState(readFolded);
  function toggle() {
    setFolded(!folded);
    try {
      localStorage.setItem(FOLD_KEY, folded ? "0" : "1");
    } catch {
      // The choice lasts for this page only.
    }
  }
  return { folded, toggle };
}

/** The sections sidebar on a desktop: 208px, or 52px of icons when folded. */
export function SectionsSidebar({ scope }: { scope: Scope }) {
  const sections = useSections(scope);
  const { folded, toggle } = useFold();
  return (
    <nav
      aria-label="Sections"
      data-folded={folded || undefined}
      className={cn(
        "flex shrink-0 flex-col gap-px overflow-x-hidden overflow-y-auto border-r border-line bg-sheet py-3 transition-[width] duration-(--t-fast) ease-out-quint",
        folded ? "w-(--side-w-folded) items-center px-2" : "w-(--side-w) px-2.5",
      )}
    >
      {sections.map(({ section, count }) => {
        const Icon = ICONS[section];
        const lit = scope.section === section;
        return (
          <Link
            key={section}
            {...sectionLink(scope, section)}
            activeOptions={{ exact: true }}
            activeProps={{}}
            aria-current={lit ? "page" : undefined}
            aria-label={`${SECTION_LABELS[section]}${countLabel(count)}`}
            title={folded ? SECTION_LABELS[section] : undefined}
            className={cn(
              "relative flex h-8 shrink-0 items-center gap-2.5 rounded-control font-medium whitespace-nowrap text-ink-2",
              "transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none",
              "aria-[current=page]:bg-selected aria-[current=page]:text-ink",
              folded ? "w-9 justify-center" : "px-2.5",
            )}
          >
            <Icon aria-hidden strokeWidth={2} className={cn("size-(--icon) shrink-0", lit ? "text-ink" : "text-ink-3")} />
            {!folded && <span className="min-w-0 flex-1 truncate">{SECTION_LABELS[section]}</span>}
            {!folded && count.waiting ? <WaitingCount count={count.waiting} /> : null}
            {!folded && !count.waiting && count.count !== undefined && (
              <span className="text-meta font-normal text-ink-3 tabular-nums">{count.count}</span>
            )}
            {folded && !!count.waiting && <WaitingCount count={count.waiting} className="absolute -top-0.5 -right-1 h-3.5 min-w-3.5 px-1 text-[9.5px]" />}
          </Link>
        );
      })}
      <button
        type="button"
        onClick={toggle}
        aria-label={folded ? "Show section names" : "Fold the sidebar"}
        title={folded ? "Show section names" : "Fold the sidebar"}
        className={cn(
          "mt-auto flex h-8 shrink-0 items-center gap-2.5 rounded-control text-ink-3 transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none",
          folded ? "w-9 justify-center" : "px-2.5",
        )}
      >
        {folded ? <ChevronsRight aria-hidden className="size-(--icon)" strokeWidth={2} /> : <PanelLeftClose aria-hidden className="size-(--icon)" strokeWidth={2} />}
        {!folded && <span className="text-meta">Fold</span>}
      </button>
    </nav>
  );
}

/** What the sections sheet says beside a section on the phone. */
function note(count: SectionCount): string | undefined {
  if (count.note) return count.note;
  return count.count !== undefined ? String(count.count) : undefined;
}

/** The phone's section button, naming the open section, with a dark count when something waits in any section, and its sheet. */
export function SectionButton({ scope, title }: { scope: Scope; title: string }) {
  const sections = useSections(scope);
  const [open, setOpen] = useState(false);
  const waiting = waitingIn(sections);
  const current = scope.section ? SECTION_LABELS[scope.section] : "Sections";
  return (
    <>
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Sections: ${current}${waiting ? `, ${waiting} waiting on you` : ""}`}
        onClick={() => setOpen(true)}
        className="flex h-11 min-w-0 flex-1 items-center gap-2.5 rounded-card border border-line bg-sheet px-3.5 text-em font-semibold text-ink shadow-control transition-colors duration-(--t-fast) ease-out-quint active:bg-hover focus-visible:shadow-focus focus-visible:outline-none"
      >
        {scope.section ? (
          (() => {
            const Icon = ICONS[scope.section];
            return <Icon aria-hidden strokeWidth={2} className="size-(--icon) shrink-0 text-ink-3" />;
          })()
        ) : null}
        <span className="min-w-0 truncate">{current}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {waiting > 0 && <WaitingCount count={waiting} />}
          <ChevronUp aria-hidden className="size-(--icon) text-ink-3" strokeWidth={2} />
        </span>
      </button>
      {open && (
        <BottomSheet label="Sections" onClose={() => setOpen(false)}>
          <p className="truncate pt-1 pb-1.5 text-meta font-medium text-ink-3">{title}</p>
          <ul className="pb-1">
            {sections.map(({ section, count }) => {
              const Icon = ICONS[section];
              const said = note(count);
              return (
                <li key={section}>
                  <Link
                    {...sectionLink(scope, section)}
                    activeOptions={{ exact: true }}
                    activeProps={{}}
                    aria-current={scope.section === section ? "page" : undefined}
                    className="-mx-2 flex h-12 items-center gap-3 rounded-card px-2 text-[15px] font-medium text-ink outline-none active:bg-hover focus-visible:bg-hover aria-[current=page]:bg-selected"
                  >
                    <Icon aria-hidden strokeWidth={2} className="size-(--icon) shrink-0 text-ink-3" />
                    <span className="min-w-0 flex-1 truncate">{SECTION_LABELS[section]}</span>
                    {count.waiting ? (
                      <span className="inline-flex h-[18px] items-center rounded-full bg-ink px-2 text-[11px] font-semibold text-on-fill">{said}</span>
                    ) : (
                      said && <span className="text-meta font-normal text-ink-3 tabular-nums">{said}</span>
                    )}
                  </Link>
                </li>
              );
            })}
          </ul>
        </BottomSheet>
      )}
    </>
  );
}
