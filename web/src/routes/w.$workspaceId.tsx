import { useState } from "react";
import { Outlet, createFileRoute, notFound, redirect, useNavigate } from "@tanstack/react-router";
import { loadSession, logOut } from "@/api/auth";
import { Button } from "@/components/ui/button";
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
    return { user: session.user, workspace };
  },
  component: WorkspaceLayout,
  notFoundComponent: WorkspaceNotFound,
});

function WorkspaceLayout() {
  const { user, workspace } = Route.useRouteContext();
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
    <LiveProvider workspaceId={workspace.id}>
      <header className="flex items-center justify-between border-b px-6 py-3">
        <div className="flex items-baseline gap-3">
          <span className="font-semibold">OmniState</span>
          <span className="text-sm text-muted-foreground">{workspace.name}</span>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-sm text-muted-foreground">{user.email}</span>
          <Button variant="outline" size="sm" onClick={logOutNow} disabled={busy}>
            Log out
          </Button>
        </div>
      </header>
      <Outlet />
    </LiveProvider>
  );
}

function WorkspaceNotFound() {
  return (
    <main className="mx-auto max-w-md p-8">
      <h1 className="text-xl font-semibold">Workspace not found</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        There is no such workspace, or your account is not a member of it.
      </p>
    </main>
  );
}
