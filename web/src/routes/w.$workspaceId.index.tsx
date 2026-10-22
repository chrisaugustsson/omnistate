import { Link, createFileRoute } from "@tanstack/react-router";
import { ArrowUpRight, FolderKanban } from "lucide-react";
import { loadProjects } from "@/api/home";
import { PageHeader } from "@/components/page-header";
import { timeAgo } from "@/lib/format";

export const Route = createFileRoute("/w/$workspaceId/")({
  loader: ({ params }) => loadProjects(params.workspaceId),
  component: WorkspaceHome,
});

function WorkspaceHome() {
  const { workspace } = Route.useRouteContext();
  const projects = Route.useLoaderData();
  return (
    <main className="mx-auto max-w-5xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow="Workspace"
        title={workspace.name}
        description="Every product your agents are planning. Open one to see what waits for you."
      />
      {projects.length === 0 ? (
        <div className="mt-10 rounded-xl border border-dashed px-6 py-12 text-center">
          <FolderKanban className="mx-auto size-6 text-muted-foreground" />
          <p className="mt-3 text-sm text-muted-foreground">No projects yet. An agent creates them through the API.</p>
        </div>
      ) : (
        <ul className="mt-10 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {projects.map((project) => (
            <li key={project.id}>
              <Link
                to="/w/$workspaceId/projects/$projectId"
                params={{ workspaceId: workspace.id, projectId: project.id }}
                aria-label={project.name}
                className="group flex h-full flex-col rounded-xl border bg-card p-5 transition-all hover:-translate-y-0.5 hover:shadow-panel"
              >
                <div className="flex items-start justify-between">
                  <span
                    aria-hidden
                    className="flex size-10 items-center justify-center rounded-xl bg-signal font-display text-2xl leading-none text-signal-foreground"
                  >
                    {project.name.slice(0, 1).toUpperCase()}
                  </span>
                  <ArrowUpRight className="size-4 text-muted-foreground/50 transition-colors group-hover:text-foreground" />
                </div>
                <span className="mt-6 block font-display text-2xl leading-tight">{project.name}</span>
                <span className="mt-1 block text-xs text-muted-foreground">Started {timeAgo(project.created_at)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </main>
  );
}
