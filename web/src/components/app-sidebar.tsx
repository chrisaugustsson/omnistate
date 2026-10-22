import { useState, type ReactNode } from "react";
import { Link, useNavigate, useParams, useRouterState } from "@tanstack/react-router";
import {
  FileText,
  GitPullRequestArrow,
  Inbox,
  LayoutDashboard,
  LayoutGrid,
  LogOut,
  MessagesSquare,
} from "lucide-react";
import { logOut } from "@/api/auth";
import { AgentAvatar } from "@/components/agent-chip";
import { Button } from "@/components/ui/button";
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

/** The section of the project that `pathname` is in: "" for the overview, else its first segment. */
function sectionOf(pathname: string, projectId: string): string {
  const rest = pathname.split(`/projects/${projectId}`)[1] ?? "";
  return rest.split("/").filter(Boolean)[0] ?? "";
}

/** The navigation of one project: its sections, with counts, the current one marked. */
function ProjectNav({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { home } = useHome(workspaceId, projectId);
  const name = useProjectName(workspaceId, projectId);
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const current = sectionOf(pathname, projectId);
  const counts: Record<string, number | undefined> = {
    inbox: home ? waitingCount(home) : undefined,
    docs: home?.docs.length,
    changesets: home?.changesets.length,
    threads: home?.threads.length,
  };

  return (
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
          const active = current === section;
          return (
            <Link
              key={section || "overview"}
              to={to}
              params={{ workspaceId, projectId }}
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
