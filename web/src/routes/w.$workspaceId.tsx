import { Outlet, createFileRoute, notFound, redirect } from "@tanstack/react-router";
import { loadSession } from "@/api/auth";
import { TopBar } from "@/components/shell/top-bar";
import { LiveProvider } from "@/live/context";

// Every page of a workspace is under /w/<workspace id>/. The layout checks the
// session and that the user is a member of that workspace.
export const Route = createFileRoute("/w/$workspaceId")({
  ssr: false,
  beforeLoad: async ({ params }) => {
    const session = await loadSession();
    if (!session) throw redirect({ to: "/login" });
    const workspace = session.workspaces.find((candidate) => candidate.id === params.workspaceId);
    if (!workspace) throw notFound();
    return { user: session.user, workspace, workspaces: session.workspaces };
  },
  component: WorkspaceLayout,
  notFoundComponent: WorkspaceNotFound,
});

// The shell (navigation D): the top bar, and under it the page on one white
// surface. The page area is the page's scroll container, so the viewers and
// the Behaviour screen can fill its height. The routes say which scope the
// bar shows and which pill is lit (components/shell/scope.ts).
function WorkspaceLayout() {
  const { user, workspace, workspaces } = Route.useRouteContext();
  return (
    <LiveProvider workspaceId={workspace.id}>
      <div className="flex h-dvh flex-col bg-sheet">
        <TopBar workspace={workspace} workspaces={workspaces} user={user} />
        <div data-slot="page" className="relative min-h-0 flex-1 overflow-y-auto">
          <Outlet />
        </div>
      </div>
    </LiveProvider>
  );
}

function WorkspaceNotFound() {
  return (
    <main className="mx-auto max-w-md p-8">
      <h1 className="text-title font-semibold">Workspace not found</h1>
      <p className="mt-2 text-ink-2">There is no such workspace, or your account is not a member of it.</p>
    </main>
  );
}
