import type { ReactNode } from "react";
import type { HomeResponse } from "@/api/generated";
import { PageHeader } from "@/components/page-header";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { useHome, useProjectName } from "@/lib/use-home";

/**
 * The frame of a project's section page (Inbox, Threads): the project's name
 * over the section's title, and the section's content once the project's home
 * is loaded (live, as on the scope pages).
 */
export function SectionPage({
  workspaceId,
  projectId,
  title,
  description,
  children,
}: {
  workspaceId: string;
  projectId: string;
  title: string;
  description?: (home: HomeResponse) => ReactNode;
  children: (home: HomeResponse) => ReactNode;
}) {
  const loaded = useHome(workspaceId, projectId);
  const { failed } = loaded;
  const { home } = loaded;
  const name = useProjectName(workspaceId, projectId);
  return (
    <main className="mx-auto max-w-5xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader
        eyebrow={name ?? "Project"}
        title={title}
        description={home && description?.(home)}
      />
      {failed && (
        <Alert variant="destructive" className="mt-8">
          <AlertDescription>The project could not be loaded. Try again in a moment.</AlertDescription>
        </Alert>
      )}
      {home && <div className="mt-8 space-y-8">{children(home)}</div>}
      {!home && !failed && <Skeleton />}
    </main>
  );
}

function Skeleton() {
  return (
    <div className="mt-8 space-y-3" aria-hidden>
      {[0, 1, 2, 3].map((row) => (
        <div key={row} className="h-14 animate-pulse rounded-xl bg-muted" />
      ))}
    </div>
  );
}
