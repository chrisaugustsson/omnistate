import { createFileRoute } from "@tanstack/react-router";
import { ForYou } from "@/components/home/for-you";

// Home's For you: what waits on you and your tracks, across the workspace's
// projects (components/home/for-you.tsx).
export const Route = createFileRoute("/w/$workspaceId/_home/")({
  staticData: { tab: "for-you" },
  component: Page,
});

function Page() {
  const { workspaceId } = Route.useParams();
  return <ForYou workspaceId={workspaceId} />;
}
