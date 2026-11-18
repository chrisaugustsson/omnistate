import { useCallback, useMemo, useState, type ReactNode } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronDown, ChevronLeft, Inbox, Search } from "lucide-react";
import { Palette, isMac, useSearchShortcut } from "@/components/palette/palette";
import { CreatePartForm, StartTrackForm } from "@/components/project/forms";
import { useMe, useProjects, useWorkspaceHomes, waitingOnMe } from "@/lib/use-workspace";
import { cn } from "@/lib/utils";
import { AccountMenu } from "./account-menu";
import { Kbd } from "./kbd";
import { PathCrumbs, SiblingsSheet, upLink, useCrumbs, useOpenParts, usePartFeatures, type PathData, type Workspace } from "./path";
import { workspaceLink, useScope } from "./scope";
import { LevelCounts, SectionButton, SectionsSidebar } from "./sections";
import { useIsPhone, useReportedCanvas, useSetHeaderActions } from "./shell-context";

// The shell of navigation F (tasks/ui-redesign/build-f.md): on a desktop, the
// path header (logo, the path, then Search, the Inbox and your account) over
// the sections sidebar and the page; on the phone, a header with ‹ to the
// level above and the open step's name (its ⌄ opens its siblings), the page,
// and a bottom bar with the section button and the Inbox. The routes say where
// the reader is (./scope.ts); the page is the shell's scroll container.

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

const iconButton =
  "relative grid size-(--control-h) shrink-0 place-items-center rounded-control text-ink-2 transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover hover:text-ink focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected";

/** The count on the Inbox icon. */
function Badge({ count }: { count: number }) {
  return (
    <span
      aria-hidden
      className="absolute top-0 -right-[3px] h-[15px] min-w-[15px] rounded-full bg-ink px-1 text-center text-[10px] leading-[15px] font-semibold text-on-fill tabular-nums shadow-[0_0_0_2px_var(--sheet)]"
    >
      {count}
    </span>
  );
}

function InboxLink({ workspaceId, waiting, className }: { workspaceId: string; waiting: number | null; className?: string }) {
  return (
    <Link
      {...workspaceLink(workspaceId, "inbox")}
      activeOptions={{ exact: true }}
      activeProps={{}}
      aria-label={waiting ? `Inbox, ${waiting} waiting` : "Inbox"}
      title="Inbox"
      className={cn(iconButton, className)}
    >
      <Inbox aria-hidden className="size-(--icon)" strokeWidth={2.25} />
      {!!waiting && <Badge count={waiting} />}
    </Link>
  );
}

/** Where the header's page actions go (HeaderActions). */
function ActionsSlot() {
  const setActions = useSetHeaderActions();
  return <div ref={setActions} className="flex shrink-0 items-center gap-1 empty:hidden" />;
}

export function Shell({
  workspace,
  workspaces,
  user,
  children,
}: {
  workspace: Workspace;
  workspaces: readonly Workspace[];
  user: { id: string; email: string };
  children: ReactNode;
}) {
  const said = useScope();
  // A page that is a canvas only sometimes says so at run time (useReportCanvas).
  const reportedCanvas = useReportedCanvas();
  const scope = reportedCanvas && !said.canvas ? { ...said, canvas: true } : said;
  const phone = useIsPhone();
  const { projects } = useProjects(workspace.id, scope.projectId);
  const { homes } = useWorkspaceHomes(workspace.id, projects);
  const me = useMe(workspace.id);
  const waiting = waitingOnMe(homes, me);
  const parts = useOpenParts(workspace.id, scope.projectId);
  const home = scope.projectId ? homes?.get(scope.projectId) : undefined;
  const track = scope.level === "track" ? home?.tracks?.find((summary) => summary.track.id === scope.trackId) : undefined;
  const partId = scope.level === "track" ? track?.track.part_id : scope.partId;
  const features = usePartFeatures(workspace.id, scope.projectId, partId, scope.level === "feature");
  const [creating, setCreating] = useState<"part" | "track" | null>(null);

  const data: PathData = {
    scope,
    workspace,
    workspaces,
    projects,
    project: projects?.find((candidate) => candidate.id === scope.projectId),
    parts,
    partId,
    part: partId ? parts?.find((candidate) => candidate.id === partId) : undefined,
    home,
    features,
    feature: features?.find((candidate) => candidate.id === scope.featureId),
    track,
    onCreate: setCreating,
  };
  const crumbs = useCrumbs(data);

  const [searching, setSearching] = useState(false);
  const closeSearch = useCallback(() => setSearching(false), []);
  useSearchShortcut(useCallback(() => setSearching((open) => !open), []));
  // The palette searches the open project; on the workspace, every project of it.
  const searched = useMemo(() => {
    if (!homes) return null;
    if (!scope.projectId) return [...homes.values()];
    const open = homes.get(scope.projectId);
    return open ? [open] : null;
  }, [homes, scope.projectId]);

  const dialogs = (
    <>
      <Palette homes={searched} projects={projects} workspaceId={workspace.id} open={searching} onClose={closeSearch} />
      {creating === "part" && scope.projectId && (
        <CreatePartForm workspaceId={workspace.id} projectId={scope.projectId} onDone={() => setCreating(null)} />
      )}
      {creating === "track" && scope.projectId && parts && (
        <StartTrackForm workspaceId={workspace.id} projectId={scope.projectId} fixedPartId={partId} parts={parts} onDone={() => setCreating(null)} />
      )}
    </>
  );
  const counts = <LevelCounts scope={scope} projects={projects} parts={parts} homes={homes} me={me} waiting={waiting} />;

  if (phone) {
    return (
      <div className="flex h-dvh flex-col bg-sheet">
        {counts}
        <PhoneHeader data={data} crumbs={crumbs} user={user} onSearch={() => setSearching(true)} />
        <div data-slot="page" className="relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto">
          {children}
        </div>
        <div className="flex shrink-0 items-start gap-2.5 border-t border-line bg-sheet px-4 pt-2.5 pb-[max(12px,env(safe-area-inset-bottom))]">
          <SectionButton scope={scope} title={crumbs[crumbs.length - 1].name} />
          <Link
            {...workspaceLink(workspace.id, "inbox")}
            activeOptions={{ exact: true }}
            activeProps={{}}
            aria-label={waiting ? `Inbox, ${waiting} waiting` : "Inbox"}
            className="relative grid size-11 shrink-0 place-items-center rounded-card border border-line bg-sheet text-ink-2 shadow-control active:bg-hover focus-visible:shadow-focus focus-visible:outline-none aria-[current=page]:bg-selected"
          >
            <Inbox aria-hidden className="size-[18px]" strokeWidth={2.25} />
            {!!waiting && <Badge count={waiting} />}
          </Link>
        </div>
        {dialogs}
      </div>
    );
  }

  return (
    <div className="flex h-dvh flex-col bg-sheet">
      {counts}
      <header className="relative z-30 flex h-(--bar-h) shrink-0 items-center gap-2 border-b border-line bg-sheet pr-3 pl-3">
        <Link
          {...workspaceLink(workspace.id, "inbox")}
          activeOptions={{ exact: true }}
          activeProps={{}}
          aria-label="OmniState, Inbox"
          title="Inbox"
          className="mr-1 grid size-8 shrink-0 place-items-center rounded-control text-ink transition-colors duration-(--t-fast) ease-out-quint hover:bg-hover focus-visible:shadow-focus focus-visible:outline-none"
        >
          <Logo />
        </Link>
        <PathCrumbs crumbs={crumbs} data={data} />
        <div className="ml-auto flex shrink-0 items-center gap-1 pl-2">
          <ActionsSlot />
          <button
            type="button"
            aria-label="Search"
            aria-keyshortcuts={isMac() ? "Meta+K" : "Control+K"}
            onClick={() => setSearching(true)}
            className="mr-1 flex h-(--control-h) w-[232px] items-center gap-2 rounded-control border border-line-strong bg-sheet pr-1 pl-2 text-left text-ink-3 shadow-control transition-colors duration-(--t-fast) ease-out-quint hover:border-faint focus-visible:shadow-focus focus-visible:outline-none max-[1279px]:w-auto max-[1279px]:border-transparent max-[1279px]:shadow-none max-[1279px]:hover:bg-hover"
          >
            <Search aria-hidden className="size-(--icon-sm) shrink-0 text-faint max-[1279px]:text-ink-2" strokeWidth={2.25} />
            <span className="min-w-0 flex-1 truncate max-[1279px]:hidden">Search</span>
            <Kbd className="max-[1279px]:hidden">{isMac() ? "⌘K" : "Ctrl K"}</Kbd>
          </button>
          <InboxLink workspaceId={workspace.id} waiting={waiting} />
          <AccountMenu user={user} workspace={workspace} workspaces={workspaces} />
        </div>
      </header>
      <div className="flex min-h-0 flex-1">
        <SectionsSidebar scope={scope} />
        <div data-slot="page" className="relative min-h-0 min-w-0 flex-1 overflow-y-auto">
          {children}
        </div>
      </div>
      {dialogs}
    </div>
  );
}

/** The phone's header: ‹ to the level above, the open step's name with ⌄ (its siblings), Search and your account. */
function PhoneHeader({
  data,
  crumbs,
  user,
  onSearch,
}: {
  data: PathData;
  crumbs: ReturnType<typeof useCrumbs>;
  user: { id: string; email: string };
  onSearch: () => void;
}) {
  const [open, setOpen] = useState(false);
  const up = upLink(data);
  const current = crumbs[crumbs.length - 1];
  return (
    <header className="relative z-30 flex h-[52px] shrink-0 items-center gap-1 border-b border-line bg-sheet px-2">
      {up && (
        <Link {...up.link} activeOptions={{ exact: true }} activeProps={{}} aria-label={`Up to ${up.name}`} className={cn(iconButton, "size-9")}>
          <ChevronLeft aria-hidden className="size-5" strokeWidth={2.25} />
        </Link>
      )}
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(true)}
        className={cn(
          "flex h-9 min-w-0 items-center gap-1.5 rounded-control pr-1.5 text-[17px] font-semibold text-ink active:bg-hover focus-visible:shadow-focus focus-visible:outline-none",
          up ? "pl-1" : "pl-2",
        )}
      >
        <span className="min-w-0 truncate">{current.name || " "}</span>
        <span className="grid size-6 shrink-0 place-items-center rounded-control bg-hover text-ink-2">
          <ChevronDown aria-hidden className="size-3.5" strokeWidth={2.5} />
        </span>
      </button>
      <div className="ml-auto flex shrink-0 items-center gap-0.5">
        <ActionsSlot />
        <button type="button" aria-label="Search" onClick={onSearch} className={cn(iconButton, "size-9")}>
          <Search aria-hidden className="size-[18px]" strokeWidth={2.25} />
        </button>
        <AccountMenu user={user} workspace={data.workspace} workspaces={data.workspaces} />
      </div>
      {open && <SiblingsSheet crumbs={crumbs} data={data} onClose={() => setOpen(false)} />}
    </header>
  );
}
