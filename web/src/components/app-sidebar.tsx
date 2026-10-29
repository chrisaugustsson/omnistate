import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Link, useNavigate, useParams, useRouterState } from "@tanstack/react-router";
import {
  Check,
  ChevronRight,
  ChevronsUpDown,
  Inbox,
  KeyRound,
  Layers,
  LayoutGrid,
  LogOut,
  MessagesSquare,
  Search,
} from "lucide-react";
import { logOut } from "@/api/auth";
import { loadProjects } from "@/api/home";
import { AgentAvatar } from "@/components/agent-chip";
import { Palette, isMac, useSearchShortcut } from "@/components/palette/palette";
import { Button } from "@/components/ui/button";
import type { HomeResponse, ProjectBody } from "@/api/generated";
import { waitingCount } from "@/components/project/items";
import { ScopeTree, useFold } from "@/components/scope-tree";
import { useReloading } from "@/lib/reloading";
import { useHome } from "@/lib/use-home";
import { useVisits, type Visit } from "@/lib/visits";
import { cn } from "@/lib/utils";

type Workspace = { id: string; name: string };
type User = { email: string };

/** The workspace's mark: a square with its first letter. */
function WorkspaceMark({ name }: { name: string }) {
  return (
    <span
      aria-hidden
      className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-primary font-display text-lg leading-none text-primary-foreground"
    >
      {name.slice(0, 1).toUpperCase()}
    </span>
  );
}

const itemClass =
  "group flex h-8 items-center gap-2.5 rounded-md px-2 text-[13px] font-medium text-sidebar-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-muted-foreground";

/** A count at the end of an item: hidden at zero unless `zero`, and while it is unknown. */
function Count({ value, signal, zero }: { value?: number; signal?: boolean; zero?: boolean }) {
  if (value === undefined || (value === 0 && !zero)) return null;
  return (
    <span
      className={cn(
        "ml-auto min-w-5 rounded-full px-1.5 text-center text-[11px] leading-5 tabular-nums",
        signal ? "bg-signal text-signal-foreground" : "text-muted-foreground",
      )}
    >
      {value}
    </span>
  );
}

const activeLink = { exact: true, includeSearch: false } as const;
const activeClass = "bg-sidebar-accent text-foreground [&>svg]:text-foreground";

/** The places of a project: Search, Inbox with what waits on the reviewer, App, Features with their number, and Threads. No feature is listed. */
function ProjectNav({
  workspaceId,
  projectId,
  home,
  onSearch,
}: {
  workspaceId: string;
  projectId: string;
  home: HomeResponse | null;
  onSearch: () => void;
}) {
  const params = { workspaceId, projectId };
  return (
    <div className="space-y-0.5">
      <button type="button" aria-keyshortcuts={isMac() ? "Meta+K" : "Control+K"} onClick={onSearch} className={cn(itemClass, "w-full text-left")}>
        <Search />
        Search
        <kbd className="ml-auto font-sans text-[11px] font-normal text-muted-foreground">{isMac() ? "⌘K" : "Ctrl K"}</kbd>
      </button>
      <Link
        to="/w/$workspaceId/projects/$projectId/inbox"
        params={params}
        activeOptions={activeLink}
        activeProps={{ className: activeClass }}
        className={itemClass}
      >
        <Inbox />
        Inbox
        <Count value={home ? waitingCount(home) : undefined} signal />
      </Link>
      <Link
        to="/w/$workspaceId/projects/$projectId/app"
        params={params}
        activeOptions={activeLink}
        activeProps={{ className: activeClass }}
        className={itemClass}
      >
        <LayoutGrid />
        App
      </Link>
      <Link
        to="/w/$workspaceId/projects/$projectId/features"
        params={params}
        activeOptions={activeLink}
        activeProps={{ className: activeClass }}
        className={itemClass}
      >
        <Layers />
        Features
        <Count value={home ? (home.features ?? []).length : undefined} zero />
      </Link>
      <Link
        to="/w/$workspaceId/projects/$projectId/threads"
        params={params}
        activeOptions={activeLink}
        activeProps={{ className: activeClass }}
        className={itemClass}
      >
        <MessagesSquare />
        Threads
      </Link>
    </div>
  );
}

const MOST_RECENT = 5;

/** The features of the visits that the project still has, newest first and each once: the last five. */
function recentFeatures(home: HomeResponse, visits: Visit[]): { id: string; name: string }[] {
  const names = new Map((home.features ?? []).map(({ feature }) => [feature.id, feature.name]));
  const ids = [...new Set(visits.map((visit) => visit.scope))].filter((scope) => names.has(scope));
  return ids.slice(0, MOST_RECENT).map((id) => ({ id, name: names.get(id) ?? "" }));
}

/** The features visited last, for getting back to them; the label folds the list. */
function Recent({ workspaceId, projectId, home, visits }: { workspaceId: string; projectId: string; home: HomeResponse; visits: Visit[] }) {
  const [open, toggle] = useFold(`omni:fold:${projectId}:recent`);
  const features = recentFeatures(home, visits);
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        onClick={toggle}
        className="flex h-[26px] w-full items-center gap-1 rounded-md px-2 text-left text-[11.5px] font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        Recent
        <ChevronRight aria-hidden className={cn("size-3 shrink-0 transition-transform", open && "rotate-90")} />
      </button>
      {open &&
        (features.length === 0 ? (
          <p className="px-2 py-1 text-xs text-muted-foreground">No recent features</p>
        ) : (
          <div className="space-y-px">
            {features.map((feature) => (
              <Link
                key={feature.id}
                to="/w/$workspaceId/projects/$projectId/features/$featureId"
                params={{ workspaceId, projectId, featureId: feature.id }}
                activeOptions={activeLink}
                activeProps={{ className: activeClass }}
                className={itemClass}
              >
                <Layers />
                <span className="min-w-0 flex-1 truncate">{feature.name}</span>
              </Link>
            ))}
          </div>
        ))}
    </div>
  );
}

/**
 * The scope that the sidebar's tree shows: the feature or App of the current
 * page, and on a page of no scope (Inbox, Features, Threads) the last one
 * visited. It is App when there is none, or when the feature is gone.
 */
function treeScope(
  home: HomeResponse,
  visits: Visit[],
  at: { featureId?: string; docId?: string; threadId?: string; onApp: boolean },
): string {
  let scope: string | undefined;
  if (at.featureId) scope = at.featureId;
  else if (at.docId) scope = scopeOf(home.docs.find((doc) => doc.id === at.docId));
  else if (at.threadId) scope = scopeOf(home.threads.find((thread) => thread.id === at.threadId));
  else if (at.onApp) scope = "app";
  scope ??= visits[0]?.scope ?? "app";
  const exists = scope === "app" || (home.features ?? []).some(({ feature }) => feature.id === scope);
  return exists ? scope : "app";
}

function scopeOf(item?: { feature_id?: string | null }): string | undefined {
  return item ? (item.feature_id ?? "app") : undefined;
}

/** What the sidebar shows of a project: its places (and the palette they open), the features visited last, and the tree of the current scope. */
function ProjectSection({
  workspaceId,
  projectId,
  onCloseDrawer,
}: {
  workspaceId: string;
  projectId: string;
  onCloseDrawer: () => void;
}) {
  const { home } = useHome(workspaceId, projectId);
  const [searching, setSearching] = useState(false);
  const closeSearch = useCallback(() => setSearching(false), []);
  useSearchShortcut(useCallback(() => setSearching((open) => !open), []));
  const visits = useVisits(projectId);
  const { featureId, docId, threadId } = useParams({ strict: false }) as {
    featureId?: string;
    docId?: string;
    threadId?: string;
  };
  const onApp = useRouterState({ select: (state) => state.location.pathname.replace(/\/$/, "").endsWith("/app") });
  return (
    <div className="space-y-3">
      <ProjectNav
        workspaceId={workspaceId}
        projectId={projectId}
        home={home}
        onSearch={() => {
          setSearching(true);
          onCloseDrawer();
        }}
      />
      <Palette home={home} workspaceId={workspaceId} projectId={projectId} open={searching} onClose={closeSearch} />
      {home && <Recent workspaceId={workspaceId} projectId={projectId} home={home} visits={visits} />}
      {home && (
        <ScopeTree
          workspaceId={workspaceId}
          projectId={projectId}
          scope={treeScope(home, visits, { featureId, docId, threadId, onApp })}
          home={home}
        />
      )}
    </div>
  );
}

const menuItemClass =
  "flex h-8 items-center gap-2 rounded-md px-2 text-[13px] text-foreground transition-colors hover:bg-accent focus-visible:bg-accent focus-visible:outline-none";

function MenuLabel({ children }: { children: ReactNode }) {
  return (
    <div className="px-2 pt-2 pb-1 text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
      {children}
    </div>
  );
}

/**
 * The header: the workspace name above the project's, and the menu of the
 * workspace's projects, All projects, and the account's workspaces when it
 * has more than one. Outside a project it shows the workspace name only.
 */
function Switcher({
  workspace,
  workspaces,
  projectId,
  projects,
  open,
  onOpen,
  onClose,
}: {
  workspace: Workspace;
  workspaces: Workspace[];
  projectId?: string;
  projects: ProjectBody[];
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const project = projects.find((candidate) => candidate.id === projectId);
  const mark = (project?.name ?? workspace.name).slice(0, 1).toUpperCase();

  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) onClose();
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open, onClose]);

  return (
    <div ref={root} className="relative pb-3">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Switch project"
        onClick={open ? onClose : onOpen}
        className="flex w-full items-center gap-2.5 rounded-lg px-1 py-1 text-left transition-colors hover:bg-sidebar-accent focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none"
      >
        <WorkspaceMark name={mark} />
        <span className="min-w-0 flex-1 leading-tight">
          <span className="block truncate text-[11px] text-muted-foreground">{workspace.name}</span>
          {projectId && <span className="block truncate text-[13px] font-semibold">{project?.name ?? "Project"}</span>}
        </span>
        <ChevronsUpDown aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Projects"
          className="absolute inset-x-0 top-full z-50 max-h-[70dvh] overflow-y-auto rounded-xl border bg-popover p-1 shadow-lg"
          onClick={(event) => {
            if ((event.target as HTMLElement).closest("a")) onClose();
          }}
        >
          <MenuLabel>Projects</MenuLabel>
          {projects.map((candidate) => (
            <Link
              key={candidate.id}
              role="menuitem"
              to="/w/$workspaceId/projects/$projectId"
              params={{ workspaceId: workspace.id, projectId: candidate.id }}
              aria-current={candidate.id === projectId ? "true" : undefined}
              className={cn(menuItemClass, candidate.id === projectId && "font-medium")}
            >
              <span className="min-w-0 flex-1 truncate">{candidate.name}</span>
              {candidate.id === projectId && <Check aria-hidden className="size-3.5" />}
            </Link>
          ))}
          <Link
            role="menuitem"
            to="/w/$workspaceId"
            params={{ workspaceId: workspace.id }}
            className={cn(menuItemClass, "mt-1 border-t pt-1")}
          >
            <LayoutGrid aria-hidden className="size-3.5 text-muted-foreground" />
            All projects
          </Link>
          {workspaces.length > 1 && (
            <>
              <MenuLabel>Workspaces</MenuLabel>
              {workspaces.map((candidate) => (
                <Link
                  key={candidate.id}
                  role="menuitem"
                  to="/w/$workspaceId"
                  params={{ workspaceId: candidate.id }}
                  aria-current={candidate.id === workspace.id ? "true" : undefined}
                  className={cn(menuItemClass, candidate.id === workspace.id && "font-medium")}
                >
                  <span className="min-w-0 flex-1 truncate">{candidate.name}</span>
                  {candidate.id === workspace.id && <Check aria-hidden className="size-3.5" />}
                </Link>
              ))}
            </>
          )}
        </div>
      )}
    </div>
  );
}

/** The left column of every workspace page: the header, the project's places and the account. */
export function AppSidebar({
  workspace,
  workspaces,
  user,
  open,
  onClose,
}: {
  workspace: Workspace;
  /** Every workspace of the account; the menu lists them when there is more than one. */
  workspaces: Workspace[];
  user: User;
  /** On small screens the sidebar is a drawer, shown while `open`. */
  open: boolean;
  onClose: () => void;
}) {
  const { projectId } = useParams({ strict: false }) as { projectId?: string };
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const closeMenu = useCallback(() => setMenuOpen(false), []);
  const load = useCallback(() => loadProjects(workspace.id), [workspace.id]);
  const { value: projects, reload } = useReloading(load);

  useEffect(() => {
    if (!menuOpen && !open) return;
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (menuOpen) setMenuOpen(false);
      else onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [menuOpen, open, onClose]);

  async function logOutNow() {
    setBusy(true);
    try {
      await logOut();
    } catch {
      // A session that is already over is what logging out is after.
    }
    await navigate({ to: "/login" });
  }

  return (
    <>
      {open && (
        <div aria-hidden className="fixed inset-0 z-30 bg-foreground/20 backdrop-blur-[1px] md:hidden" onClick={onClose} />
      )}
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-40 w-64 shrink-0 flex-col bg-sidebar px-3 py-3 shadow-xl md:static md:z-auto md:flex md:w-60 md:shadow-none",
          open ? "flex" : "hidden",
        )}
        onClick={(event) => {
          if ((event.target as HTMLElement).closest("a")) onClose();
        }}
      >
        <Switcher
          workspace={workspace}
          workspaces={workspaces}
          projectId={projectId}
          projects={projects ?? []}
          open={menuOpen}
          onOpen={() => {
            reload();
            setMenuOpen(true);
          }}
          onClose={closeMenu}
        />

        <nav aria-label="Project" className="min-h-0 flex-1 overflow-y-auto">
          {projectId && <ProjectSection workspaceId={workspace.id} projectId={projectId} onCloseDrawer={onClose} />}
        </nav>

        <div className="mt-3 space-y-2 border-t pt-3">
          <div className="flex items-center gap-2 pl-1">
            <AgentAvatar name={user.email} className="size-6 text-[10px]" />
            <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{user.email}</span>
          </div>
          <div className="flex items-center gap-1">
            <Link
              to="/w/$workspaceId/api-keys"
              params={{ workspaceId: workspace.id }}
              activeProps={{ className: "bg-sidebar-accent text-foreground" }}
              className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-foreground"
            >
              <KeyRound className="size-3.5" />
              API keys
            </Link>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs text-muted-foreground"
              onClick={logOutNow}
              disabled={busy}
            >
              <LogOut className="size-3.5" />
              Log out
            </Button>
          </div>
        </div>
      </aside>
    </>
  );
}
