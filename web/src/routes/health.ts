import { createFileRoute } from "@tanstack/react-router";
import { healthResponse } from "../health";

export const Route = createFileRoute("/health")({
  server: {
    handlers: {
      GET: () => healthResponse(),
    },
  },
});
