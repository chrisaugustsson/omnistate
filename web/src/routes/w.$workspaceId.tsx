import { useState } from "react";
import { Outlet, createFileRoute, notFound, redirect } from "@tanstack/react-router";
import { Menu } from "lucide-react";
import { loadSession } from "@/api/auth";
import { AppSidebar } from "@/components/app-sidebar";
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

// The shell: the sidebar on the stone canvas, and the page on a white panel
// inset next to it. The panel is the page's scroll container, so the viewers
// can fill its height.
function WorkspaceLayout() {
  const { user, workspace } = Route.useRouteContext();
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <LiveProvider workspaceId={workspace.id}>
      <div className="flex h-dvh bg-canvas">
        <AppSidebar workspace={workspace} user={user} open={menuOpen} onClose={() => setMenuOpen(false)} />
        <div className="flex min-w-0 flex-1 flex-col md:py-2 md:pr-2">
          <div className="flex h-12 shrink-0 items-center gap-2 border-b bg-background px-3 md:hidden">
            <button
              type="button"
              aria-label="Open the menu"
              className="flex size-8 items-center justify-center rounded-md hover:bg-accent"
              onClick={() => setMenuOpen(true)}
            >
              <Menu className="size-4" />
            </button>
            <span className="text-sm font-semibold">{workspace.name}</span>
          </div>
          <div
            data-slot="page"
            className="relative min-h-0 flex-1 overflow-auto bg-background md:rounded-xl md:shadow-panel"
          >
            <Outlet />
          </div>
        </div>
      </div>
    </LiveProvider>
  );
}

function WorkspaceNotFound() {
  return (
    <main className="mx-auto max-w-md p-8">
      <h1 className="font-display text-3xl">Workspace not found</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        There is no such workspace, or your account is not a member of it.
      </p>
    </main>
  );
}
