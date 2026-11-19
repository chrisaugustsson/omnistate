import { Outlet, createFileRoute, notFound, redirect } from "@tanstack/react-router";
import { loadSession } from "@/api/auth";
import { Shell } from "@/components/shell/shell";
import { ShellProvider } from "@/components/shell/shell-context";
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

// The shell (navigation F, components/shell/shell.tsx): the path header and
// the sections sidebar around the page, or on the phone the header and the
// bottom bar. The routes say which level and section the reader is on
// (components/shell/scope.ts); the pages report their sections' counts.
function WorkspaceLayout() {
  const { user, workspace, workspaces } = Route.useRouteContext();
  return (
    <LiveProvider workspaceId={workspace.id}>
      <ShellProvider>
        <Shell workspace={workspace} workspaces={workspaces} user={user}>
          <Outlet />
        </Shell>
      </ShellProvider>
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
