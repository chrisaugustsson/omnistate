import { createFileRoute } from "@tanstack/react-router";
import { ForYou } from "@/components/home/for-you";

// The workspace's own page, its Inbox section. It shows For you (what waits on
// you and your tracks, components/home/for-you.tsx) until the Inbox replaces it.
export const Route = createFileRoute("/w/$workspaceId/_home/")({
  staticData: { section: "inbox" },
  component: Page,
});

function Page() {
  const { workspaceId } = Route.useParams();
  return <ForYou workspaceId={workspaceId} />;
}
