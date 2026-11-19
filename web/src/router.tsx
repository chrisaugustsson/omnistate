import { createRouter } from "@tanstack/react-router";
import { routeTree } from "./routeTree.gen";

// A path that matches no route, such as an old /features URL, gets the root's
// not-found screen. The default would hand it to the workspace layout, which
// renders without its route context and crashes.
export function getRouter() {
  return createRouter({ routeTree, scrollRestoration: true, notFoundMode: "root" });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof getRouter>;
  }
}
