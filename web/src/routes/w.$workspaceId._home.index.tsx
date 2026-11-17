import { createFileRoute } from "@tanstack/react-router";
import { PlaceholderPage } from "@/components/shell/page";

// Home's For you: what waits on you and your tracks, across the workspace's
// projects. Slice S4 builds it.
export const Route = createFileRoute("/w/$workspaceId/_home/")({
  staticData: { tab: "for-you" },
  component: ForYou,
});

function ForYou() {
  return <PlaceholderPage title="For you">What waits on you and your tracks, across the workspace&rsquo;s projects.</PlaceholderPage>;
}
