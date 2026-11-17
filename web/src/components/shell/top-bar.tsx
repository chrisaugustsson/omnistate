import { useCallback, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronsUpDown, Inbox, Search } from "lucide-react";
import type { HomeResponse, PartBody, ProjectBody } from "@/api/generated";
import { Palette, isMac, useSearchShortcut } from "@/components/palette/palette";
import { isProjectLevel } from "@/components/project/items";
import { partTones, personName } from "@/lib/identity";
import { useFeatures } from "@/lib/use-features";
import { useParts } from "@/lib/use-parts";
import { useMe, useProjects, useWorkspaceHomes, waitingOnMe } from "@/lib/use-workspace";
import { cn } from "@/lib/utils";
import { AccountMenu } from "./account-menu";
import { Avatar } from "./avatar";
import { PartIcon, ProjectMark } from "./part-icon";
import { PillLink, PillTabs } from "./pill-tabs";
import { usePopover } from "./popover";
import {
  HOME_TABS,
  PART_TABS,
  PROJECT_TABS,
  TAB_LABELS,
  homeLink,
  partLink,
  projectLink,
  useScope,
  type Scope,
  type Tab,
} from "./scope";
import { inFlight } from "./state-icon";
import { Kbd, Switcher } from "./switcher";

// The top bar (navigation D, tasks/ui-redesign/v2/build-brief-d.md): one 48px
// row on white with a hairline under it. The logo (to Home); the scope button
// (the scope's mark, the project above, the scope below) that opens the
// switcher; the scope's sections as pills; then Search (the ⌘K palette), the
// Inbox with what waits on you, and your account menu. The scope comes from the
// routes (./scope.ts): an object page keeps its part in the bar, its section lit.

type Workspace = { id: string; name: string };

/** The OmniState mark. */
function Logo() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden className="size-[22px]">
      <rect x="1" y="1" width="22" height="22" rx="6" fill="currentColor" />
      <circle cx="12" cy="12" r="5.25" fill="none" stroke="var(--on-fill)" strokeWidth="2.25" />
      <circle cx="17.6" cy="6.4" r="2" fill="var(--part-2)" />
    </svg>
  );
}

/** How many of `tracks` are in flight (planning or building), on the part `partId` when given. */
function tracksInFlight(home: HomeResponse | undefined, partId?: string): number | undefined {
  if (!home) return undefined;
  return (home.tracks ?? []).filter(({ track }) => inFlight(track.state) && (!partId || track.part_id === partId)).length;
}

/** A count to show on a pill: hidden while unknown and at zero. */
const shown = (count: number | undefined) => (count ? count : undefined);

type ScopeLink = ReturnType<typeof homeLink> | ReturnType<typeof projectLink> | ReturnType<typeof partLink>;

function Pill({ link, tab, current, count }: { link: ScopeLink; tab: Tab; current: boolean; count?: number }) {
  return (
    <PillLink {...link} activeOptions={{ exact: true }} activeProps={{}} aria-current={current ? "page" : undefined} count={shown(count)}>
      {TAB_LABELS[tab]}
    </PillLink>
  );
}

/** The scope button: the scope's mark, the line above (12px ink-2) and the scope (13/600), and the chevron. */
function ScopeButton({
  mark,
  above,
  name,
  open,
  onClick,
  opener,
}: {
  mark: ReactNode;
  above: string;
  name: string;
  open: boolean;
  onClick: () => void;
  opener: React.Ref<HTMLButtonElement>;
}) {
  return (
    <button
      ref={opener}
      type="button"
      aria-haspopup="dialog"
      aria-expanded={open}
      onClick={onClick}
      className={cn(
        "flex h-10 max-w-[260px] min-w-[176px] items-center gap-2.5 rounded-card pr-2 pl-1.5 text-left",
        "transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-expanded:bg-selected",
      )}
    >
      {mark}
      <span className="flex min-w-0 flex-1 flex-col leading-[1.25]">
        <span className="truncate text-meta text-ink-2">{above}</span>
        <span className="truncate font-semibold text-ink">{name || " "}</span>
      </span>
      <ChevronsUpDown aria-hidden className="ml-1 size-(--icon-sm) shrink-0 text-faint" strokeWidth={2.25} />
    </button>
  );
}

/** The pills of a part, with its features counted. */
function PartPills({
  scope,
  projectId,
  part,
  home,
}: {
  scope: Scope;
  projectId: string;
  part: PartBody;
  home: HomeResponse | undefined;
}) {
  const { features } = useFeatures(scope.workspaceId, projectId, part.id);
  const counts: Partial<Record<Tab, number | undefined>> = {
    features: features?.length,
    docs: home?.docs.filter((doc) => doc.part_id === part.id && !doc.track_id && !doc.feature_id).length,
    tracks: tracksInFlight(home, part.id),
  };
  return (
    <PillTabs label={part.name}>
      {PART_TABS.map((tab) => (
        <Pill
          key={tab}
          tab={tab}
          link={partLink(scope.workspaceId, projectId, part.id, tab)}
          current={scope.tab === tab}
          count={counts[tab]}
        />
      ))}
    </PillTabs>
  );
}

/** The scope button, its switcher and the pills, for a project or one of its parts. */
function ProjectScope({
  scope,
  workspace,
  projects,
  home,
}: {
  scope: Scope;
  workspace: Workspace;
  projects: readonly ProjectBody[] | null;
  home: HomeResponse | undefined;
}) {
  const projectId = scope.projectId!;
  const { parts } = useParts(scope.workspaceId, projectId);
  const project = projects?.find((candidate) => candidate.id === projectId);
  const tones = parts ? partTones(parts) : null;
  const partId =
    scope.kind === "track" ? home?.tracks?.find(({ track }) => track.id === scope.trackId)?.track.part_id : scope.partId;
  const part = partId ? parts?.find((candidate) => candidate.id === partId) : undefined;
  // A part page, or an object page whose part is not known yet: the part's pills wait for it.
  const onPart = scope.kind !== "project";
  const popover = usePopover();

  const marks = parts
    ? [...parts]
        .sort((a, b) => (tones?.get(a.id) ?? 0) - (tones?.get(b.id) ?? 0))
        .map((candidate) => ({ kind: candidate.kind, tone: tones?.get(candidate.id) ?? 1 }))
    : undefined;
  const mark =
    onPart && part ? (
      <PartIcon name={part.name} kind={part.kind} tone={tones?.get(part.id) ?? 1} size="md" />
    ) : onPart ? (
      <span aria-hidden className="size-(--mark-md) shrink-0 rounded-control bg-hover" />
    ) : (
      <ProjectMark name={project?.name ?? ""} parts={marks} size="md" />
    );

  const projectCounts: Partial<Record<Tab, number | undefined>> = {
    parts: parts?.length,
    docs: home?.docs.filter(isProjectLevel).length,
    tracks: tracksInFlight(home),
  };

  return (
    <>
      <div ref={popover.anchor} className="relative">
        <ScopeButton
          opener={popover.opener}
          open={popover.open}
          onClick={popover.toggle}
          mark={mark}
          above={onPart ? (project?.name ?? "") : workspace.name}
          name={onPart ? (part?.name ?? "") : (project?.name ?? "")}
        />
        {popover.open && projects && (
          <Switcher
            workspaceId={workspace.id}
            workspaceName={workspace.name}
            projects={projects}
            projectId={projectId}
            partId={onPart ? partId : undefined}
            tab={scope.tab}
            home={false}
            onClose={() => popover.setOpen(false)}
          />
        )}
      </div>
      <span aria-hidden className="mx-1 h-5 w-px shrink-0 bg-line" />
      {!onPart && project && (
        <PillTabs label={project.name}>
          {PROJECT_TABS.map((tab) => (
            <Pill
              key={tab}
              tab={tab}
              link={projectLink(scope.workspaceId, projectId, tab)}
              current={scope.tab === tab}
              count={projectCounts[tab]}
            />
          ))}
        </PillTabs>
      )}
      {onPart && part && <PartPills scope={scope} projectId={projectId} part={part} home={home} />}
    </>
  );
}

/** The scope button, its switcher and the pills on Home. */
function HomeScope({
  scope,
  workspace,
  projects,
  user,
  waiting,
}: {
  scope: Scope;
  workspace: Workspace;
  projects: readonly ProjectBody[] | null;
  user: { id: string; email: string };
  waiting: number | null;
}) {
  const popover = usePopover();
  return (
    <>
      <div ref={popover.anchor} className="relative">
        <ScopeButton
          opener={popover.opener}
          open={popover.open}
          onClick={popover.toggle}
          mark={<Avatar name={personName(user.email)} tintKey={user.id} size="md" />}
          above={workspace.name}
          name="Home"
        />
        {popover.open && projects && (
          <Switcher
            workspaceId={workspace.id}
            workspaceName={workspace.name}
            projects={projects}
            tab={scope.tab}
            home
            onClose={() => popover.setOpen(false)}
          />
        )}
      </div>
      <span aria-hidden className="mx-1 h-5 w-px shrink-0 bg-line" />
      <PillTabs label="Home">
        {HOME_TABS.map((tab) => (
          <Pill
            key={tab}
            tab={tab}
            link={homeLink(workspace.id, tab)}
            current={scope.tab === tab}
            count={tab === "inbox" ? (waiting ?? undefined) : undefined}
          />
        ))}
      </PillTabs>
    </>
  );
}

export function TopBar({
  workspace,
  workspaces,
  user,
}: {
  workspace: Workspace;
  workspaces: readonly Workspace[];
  user: { id: string; email: string };
}) {
  const scope = useScope();
  const { projects } = useProjects(workspace.id, scope.projectId);
  const { homes } = useWorkspaceHomes(workspace.id, projects);
  const me = useMe(workspace.id);
  const waiting = waitingOnMe(homes, me);
  const [searching, setSearching] = useState(false);
  const closeSearch = useCallback(() => setSearching(false), []);
  useSearchShortcut(useCallback(() => setSearching((open) => !open), []));
  // The palette searches the open project; on Home, the workspace's first.
  const searched = scope.projectId ?? projects?.[0]?.id;
  const home = scope.projectId ? homes?.get(scope.projectId) : undefined;

  return (
    <header className="relative z-30 flex h-(--bar-h) shrink-0 items-center gap-2 border-b border-line bg-sheet px-3">
      <Link
        to="/w/$workspaceId"
        params={{ workspaceId: workspace.id }}
        activeProps={{}}
        aria-label="OmniState home"
        title="Home"
        className="grid size-8 shrink-0 place-items-center rounded-control text-ink transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none"
      >
        <Logo />
      </Link>
      {scope.kind === "home" || !scope.projectId ? (
        <HomeScope scope={scope} workspace={workspace} projects={projects} user={user} waiting={waiting} />
      ) : (
        <ProjectScope key={scope.projectId} scope={scope} workspace={workspace} projects={projects} home={home} />
      )}
      <div className="ml-auto flex shrink-0 items-center gap-1">
        <button
          type="button"
          aria-keyshortcuts={isMac() ? "Meta+K" : "Control+K"}
          onClick={() => setSearching(true)}
          className="mr-1 flex h-(--control-h) w-[232px] items-center gap-2 rounded-control border border-line-strong bg-sheet pr-1 pl-2 text-left text-ink-3 shadow-control transition-colors duration-(--t-fast) ease-out-quint hover:border-faint focus-visible:shadow-focus focus-visible:outline-none max-[960px]:w-auto"
        >
          <Search aria-hidden className="size-(--icon-sm) shrink-0 text-faint" strokeWidth={2.25} />
          <span className="min-w-0 flex-1 truncate max-[960px]:hidden">Search</span>
          <Kbd>{isMac() ? "⌘K" : "Ctrl K"}</Kbd>
        </button>
        <Link
          {...homeLink(workspace.id, "inbox")}
          activeOptions={{ exact: true }}
          aria-label={waiting ? `Inbox, ${waiting} waiting` : "Inbox"}
          title="Inbox"
          className="relative grid size-(--control-h) place-items-center rounded-control text-ink-2 transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected"
        >
          <Inbox aria-hidden className="size-(--icon)" strokeWidth={2.25} />
          {!!waiting && (
            <span
              aria-hidden
              className="absolute top-0 -right-[3px] h-[15px] min-w-[15px] rounded-full bg-brand px-1 text-center text-[10px] leading-[15px] font-semibold text-on-fill tabular-nums shadow-[0_0_0_2px_var(--sheet)]"
            >
              {waiting}
            </span>
          )}
        </Link>
        <AccountMenu user={user} workspace={workspace} workspaces={workspaces} />
      </div>
      {searched && (
        <Palette
          home={homes?.get(searched) ?? null}
          workspaceId={workspace.id}
          projectId={searched}
          open={searching}
          onClose={closeSearch}
        />
      )}
    </header>
  );
}
