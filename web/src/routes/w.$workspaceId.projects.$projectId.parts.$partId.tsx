import { Outlet, createFileRoute } from "@tanstack/react-router";

// A part's pages. The bar shows the part (its mark in its colour, the project
// above it) with its pills: Overview, Features, Docs, Tracks, Decisions.
export const Route = createFileRoute("/w/$workspaceId/projects/$projectId/parts/$partId")({
  staticData: { scope: "part" },
  component: Outlet,
});
