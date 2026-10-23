import { useId, useState, type ReactNode } from "react";
import { Link, useNavigate, useParams, useRouterState } from "@tanstack/react-router";
import {
  ChevronRight,
  FileText,
  GitPullRequestArrow,
  Inbox,
  Layers,
  LayoutDashboard,
  LayoutGrid,
  LogOut,
  MessagesSquare,
} from "lucide-react";
import { logOut } from "@/api/auth";
import { AgentAvatar } from "@/components/agent-chip";
import { Button } from "@/components/ui/button";
import type { FeatureSummary, HomeResponse } from "@/api/generated";
import { waitingCount } from "@/components/project/items";
import { useHome, useProjectName } from "@/lib/use-home";
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

function Count({ value, signal }: { value?: number; signal?: boolean }) {
  if (!value) return null;
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

/** The sections of a project, each a page of its own; the detail pages belong to theirs. */
const SECTIONS = [
  { section: "", label: "Overview", icon: LayoutDashboard, to: "/w/$workspaceId/projects/$projectId" },
  { section: "inbox", label: "Inbox", icon: Inbox, to: "/w/$workspaceId/projects/$projectId/inbox" },
  { section: "docs", label: "Docs", icon: FileText, to: "/w/$workspaceId/projects/$projectId/docs" },
  {
    section: "changesets",
    label: "Change sets",
    icon: GitPullRequestArrow,
    to: "/w/$workspaceId/projects/$projectId/changesets",
  },
  { section: "threads", label: "Threads", icon: MessagesSquare, to: "/w/$workspaceId/projects/$projectId/threads" },
] as const;

/** The pages of a feature, each a page of its own under the feature. */
const FEATURE_SECTIONS = [
  { section: "docs", label: "Docs", to: "/w/$workspaceId/projects/$projectId/features/$featureId/docs" },
  {
    section: "changesets",
    label: "Change sets",
    to: "/w/$workspaceId/projects/$projectId/features/$featureId/changesets",
  },
  { section: "threads", label: "Threads", to: "/w/$workspaceId/projects/$projectId/features/$featureId/threads" },
] as const;

/** Where the reviewer is in a project: a feature (null for the project's own pages) and a section of it ("" is the overview). */
type Place = { feature: string | null; section: string };

/**
 * The place that `pathname` is in. A detail page (a doc, change set or thread)
 * belongs to the section of its kind, in the feature that it belongs to, which
 * the project's home tells.
 */
function placeOf(pathname: string, projectId: string, home: HomeResponse | null | undefined): Place {
  const rest = (pathname.split(`/projects/${projectId}`)[1] ?? "").split("/").filter(Boolean);
  if (rest[0] === "features") return { feature: rest[1] ?? null, section: rest[2] ?? "" };
  const [section = "", id] = rest;
  const items = { docs: home?.docs, changesets: home?.changesets, threads: home?.threads }[section as "docs"];
  return { feature: (id && items?.find((item) => item.id === id)?.feature_id) || null, section };
}

/** A feature in the sidebar: its overview with its waiting count, and while it is the current one, its pages. */
function FeatureItem({
  summary,
  workspaceId,
  projectId,
  place,
}: {
  summary: FeatureSummary;
  workspaceId: string;
  projectId: string;
  place: Place;
}) {
  const { id, name } = summary.feature;
  const here = place.feature === id;
  const overview = here && place.section === "";
  return (
    <li className="space-y-0.5">
      <Link
        to="/w/$workspaceId/projects/$projectId/features/$featureId"
        params={{ workspaceId, projectId, featureId: id }}
        activeOptions={{ exact: true }}
        aria-current={overview ? "page" : undefined}
        className={cn(itemClass, here && "text-foreground", overview && "bg-sidebar-accent [&>svg]:text-foreground")}
      >
        <Layers />
        <span className="min-w-0 truncate">{name}</span>
        <Count value={summary.waiting} />
        {summary.feature.status === "done" && (
          <span className={cn("text-[11px] font-medium text-muted-foreground", !summary.waiting && "ml-auto")}>Done</span>
        )}
      </Link>
      {here &&
        FEATURE_SECTIONS.map(({ section, label, to }) => {
          const active = place.section === section;
          return (
            <Link
              key={section}
              to={to}
              params={{ workspaceId, projectId, featureId: id }}
              activeOptions={{ exact: true }}
              aria-current={active ? "page" : undefined}
              className={cn(itemClass, "pl-9", active && "bg-sidebar-accent text-foreground")}
            >
              {label}
            </Link>
          );
        })}
    </li>
  );
}

/** The project's features below its sections: the active ones, and the done ones under a toggle. */
function Features({
  features,
  workspaceId,
  projectId,
  place,
}: {
  features: FeatureSummary[];
  workspaceId: string;
  projectId: string;
  place: Place;
}) {
  const titleId = useId();
  const [shown, setShown] = useState<boolean | null>(null);
  const active = features.filter((summary) => summary.feature.status !== "done");
  const done = features.filter((summary) => summary.feature.status === "done");
  // A done feature that the reviewer is in stays in sight, until the toggle is used.
  const expanded = shown ?? done.some((summary) => summary.feature.id === place.feature);
  if (!features.length) return null;
  const item = (summary: FeatureSummary) => (
    <FeatureItem key={summary.feature.id} summary={summary} workspaceId={workspaceId} projectId={projectId} place={place} />
  );

  return (
    <div role="group" aria-labelledby={titleId} className="space-y-0.5">
      <div
        id={titleId}
        className="flex h-7 items-center px-2 text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase"
      >
        Features
      </div>
      <ul className="space-y-0.5">{active.map(item)}</ul>
      {done.length > 0 && (
        <>
          <button
            type="button"
            aria-expanded={expanded}
            onClick={() => setShown(!expanded)}
            className={cn(itemClass, "w-full")}
          >
            <ChevronRight className={cn("transition-transform", expanded && "rotate-90")} />
            Done{" "}
            <Count value={done.length} />
          </button>
          {expanded && <ul className="space-y-0.5">{done.map(item)}</ul>}
        </>
      )}
    </div>
  );
}

/** The navigation of one project: its sections, with counts, and its features, the current place marked. */
function ProjectNav({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { home } = useHome(workspaceId, projectId);
  const name = useProjectName(workspaceId, projectId);
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const place = placeOf(pathname, projectId, home);
  const ownCount = (items: { feature_id?: string | null }[] | undefined) => items?.filter((item) => !item.feature_id).length;
  const counts: Record<string, number | undefined> = {
    inbox: home ? waitingCount(home) : undefined,
    docs: ownCount(home?.docs),
    changesets: ownCount(home?.changesets),
    threads: ownCount(home?.threads),
  };

  return (
    <div className="space-y-3">
      <div className="space-y-1">
        <div className="flex h-7 items-center gap-2 px-2">
          <span
            aria-hidden
            className="flex size-4 items-center justify-center rounded-[5px] bg-signal text-[9px] font-semibold text-signal-foreground"
          >
            {(name ?? "P").slice(0, 1).toUpperCase()}
          </span>
          <span className="truncate text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
            {name ?? "Project"}
          </span>
        </div>
        <div className="space-y-0.5">
          {SECTIONS.map(({ section, label, icon: Icon, to }) => {
            const active = place.feature === null && place.section === section;
            return (
              <Link
                key={section || "overview"}
                to={to}
                params={{ workspaceId, projectId }}
                activeOptions={{ exact: true }}
                aria-current={active ? "page" : undefined}
                className={cn(itemClass, active && "bg-sidebar-accent text-foreground [&>svg]:text-foreground")}
              >
                <Icon />
                {label}
                <Count value={counts[section]} signal={section === "inbox"} />
              </Link>
            );
          })}
        </div>
      </div>
      <Features features={home?.features ?? []} workspaceId={workspaceId} projectId={projectId} place={place} />
    </div>
  );
}

/** The left column of every workspace page: the workspace, the project's navigation and the account. */
export function AppSidebar({
  workspace,
  user,
  open,
  onClose,
}: {
  workspace: Workspace;
  user: User;
  /** On small screens the sidebar is a drawer, shown while `open`. */
  open: boolean;
  onClose: () => void;
}) {
  const { projectId } = useParams({ strict: false }) as { projectId?: string };
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);

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
      <div className="flex items-center gap-2.5 px-1 pb-4">
        <WorkspaceMark name={workspace.name} />
        <div className="min-w-0 leading-tight">
          <div className="truncate text-[13px] font-semibold">{workspace.name}</div>
          <div className="text-[11px] text-muted-foreground">OmniState</div>
        </div>
      </div>

      <nav aria-label="Workspace" className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto">
        <div className="space-y-0.5">
          <Link
            to="/w/$workspaceId"
            params={{ workspaceId: workspace.id }}
            className={itemClass}
            activeOptions={{ exact: true }}
            activeProps={{ className: "bg-sidebar-accent text-foreground" }}
          >
            <LayoutGrid />
            All projects
          </Link>
        </div>
        {projectId && <ProjectNav workspaceId={workspace.id} projectId={projectId} />}
      </nav>

      <SidebarFooter>
        <AgentAvatar name={user.email} className="size-6 text-[10px]" />
        <span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">{user.email}</span>
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
      </SidebarFooter>
    </aside>
    </>
  );
}

function SidebarFooter({ children }: { children: ReactNode }) {
  return <div className="mt-3 flex items-center gap-2 border-t pt-3 pl-1">{children}</div>;
}
