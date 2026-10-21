import { Link, createFileRoute } from "@tanstack/react-router";
import { loadProjects } from "@/api/home";

export const Route = createFileRoute("/w/$workspaceId/")({
  loader: ({ params }) => loadProjects(params.workspaceId),
  component: WorkspaceHome,
});

function WorkspaceHome() {
  const { workspace } = Route.useRouteContext();
  const projects = Route.useLoaderData();
  return (
    <main className="mx-auto max-w-3xl p-6">
      <h1 className="text-xl font-semibold">{workspace.name}</h1>
      {projects.length === 0 ? (
        <p className="mt-2 text-sm text-muted-foreground">
          No projects yet. An agent creates them through the API.
        </p>
      ) : (
        <ul className="mt-4 divide-y rounded-md border">
          {projects.map((project) => (
            <li key={project.id}>
              <Link
                to="/w/$workspaceId/projects/$projectId"
                params={{ workspaceId: workspace.id, projectId: project.id }}
                className="block px-4 py-3 font-medium hover:bg-muted"
              >
                {project.name}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
