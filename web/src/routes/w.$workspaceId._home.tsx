import { Outlet, createFileRoute } from "@tanstack/react-router";

// Home: the workspace's pages for you (For you, Inbox, My tracks, and your API
// keys). The bar shows your avatar and "Home" with Home's pills.
export const Route = createFileRoute("/w/$workspaceId/_home")({
  staticData: { scope: "home" },
  component: Outlet,
});
