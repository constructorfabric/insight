import { createRouter } from "@tanstack/react-router";

import { AppBootSpinner } from "@/components/app-boot-spinner";
import { routeTree } from "./routeTree.gen";

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
  defaultPendingComponent: AppBootSpinner,
  defaultPendingMs: 200,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
