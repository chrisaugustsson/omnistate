import { createFileRoute, redirect } from "@tanstack/react-router";
import { loadSession } from "@/api/auth";

// The start page sends the reviewer to the login page, or into their first
// workspace. The pages talk to the API from the browser, which holds the
// session cookie, so none of them is rendered on the server.
export const Route = createFileRoute("/")({
  ssr: false,
  beforeLoad: async () => {
    const session = await loadSession();
    if (!session) throw redirect({ to: "/login" });
    const [workspace] = session.workspaces;
    if (workspace) {
      throw redirect({ to: "/w/$workspaceId", params: { workspaceId: workspace.id } });
    }
  },
  component: NoWorkspace,
});

function NoWorkspace() {
  return (
    <main className="mx-auto max-w-md p-8">
      <h1 className="text-xl font-semibold">OmniState</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        Your account is not a member of any workspace.
      </p>
    </main>
  );
}
