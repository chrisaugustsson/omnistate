import { useCallback, useState, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { FileText, GitPullRequestArrow, Inbox, LayoutGrid, LogOut, MessagesSquare } from "lucide-react";
import { logOut } from "@/api/auth";
import { loadProjects } from "@/api/home";
import { AgentAvatar } from "@/components/agent-chip";
import { Button } from "@/components/ui/button";
import { useHome } from "@/lib/use-home";
import { useReloading } from "@/lib/reloading";
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

/** The navigation of one project: its home and the sections of the home, with counts. */
function ProjectNav({ workspaceId, projectId }: { workspaceId: string; projectId: string }) {
  const { home } = useHome(workspaceId, projectId);
  const loadAll = useCallback(() => loadProjects(workspaceId), [workspaceId]);
  const { value: projects } = useReloading(loadAll);
  const name = projects?.find((project) => project.id === projectId)?.name;
  const waiting = home
    ? home.waiting.decisions.length +
      home.waiting.changesets.length +
      home.waiting.comments.length +
      home.waiting.questions.length
    : undefined;
  const params = { workspaceId, projectId };
  const to = "/w/$workspaceId/projects/$projectId" as const;

  return (
    <div className="space-y-1">
      <div className="flex h-7 items-center gap-2 px-2 text-[11px] font-medium tracking-[0.08em] text-muted-foreground uppercase">
        Project
      </div>
      <Link
        to={to}
        params={params}
        className={cn(itemClass, "text-foreground")}
        activeOptions={{ exact: true, includeHash: false }}
        activeProps={{ className: "bg-sidebar-accent" }}
      >
        <span
          aria-hidden
          className="flex size-4 items-center justify-center rounded-[5px] bg-signal text-[9px] font-semibold text-signal-foreground"
        >
          {(name ?? "P").slice(0, 1).toUpperCase()}
        </span>
        <span className="truncate">{name ?? "Overview"}</span>
      </Link>
      <div className="ml-[15px] space-y-0.5 border-l border-border pl-2">
        <Link to={to} params={params} hash="waiting" className={itemClass}>
          <Inbox />
          Inbox
          <Count value={waiting} signal />
        </Link>
        <Link to={to} params={params} hash="docs" className={itemClass}>
          <FileText />
          Docs
          <Count value={home?.docs.length} />
        </Link>
        <Link to={to} params={params} hash="changesets" className={itemClass}>
          <GitPullRequestArrow />
          Change sets
          <Count value={home?.changesets.length} />
        </Link>
        <Link to={to} params={params} hash="threads" className={itemClass}>
          <MessagesSquare />
          Threads
          <Count value={home?.threads.length} />
        </Link>
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
