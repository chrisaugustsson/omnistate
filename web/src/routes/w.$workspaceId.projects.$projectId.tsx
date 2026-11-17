import { Outlet, createFileRoute } from "@tanstack/react-router";

// A project's pages. The bar shows the project (its mark, the workspace above
// it) with its pills: Overview, Parts, Docs, Tracks, Decisions. A part's pages
// and the project's objects (features, tracks) under it set their own scope.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId")({
  staticData: { scope: "project" },
  component: Outlet,
});
