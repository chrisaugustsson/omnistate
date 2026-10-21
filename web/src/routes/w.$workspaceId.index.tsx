import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/w/$workspaceId/")({
  component: WorkspaceHome,
});

function WorkspaceHome() {
  const { workspace } = Route.useRouteContext();
  return (
    <main className="mx-auto max-w-3xl p-6">
      <h1 className="text-xl font-semibold">{workspace.name}</h1>
      <p className="mt-2 text-sm text-muted-foreground">Projects will show up here.</p>
    </main>
  );
}
