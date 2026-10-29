import { createFileRoute } from "@tanstack/react-router";
import { PageHeader } from "@/components/page-header";

// The Features page. A placeholder until the page with the list of features replaces it.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId_/features/")({
  component: Features,
});

function Features() {
  return (
    <main className="mx-auto max-w-6xl px-5 py-8 md:px-10 md:py-12">
      <PageHeader title="Features" />
    </main>
  );
}
