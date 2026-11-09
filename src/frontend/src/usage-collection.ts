import type { AnyRouter } from "@tanstack/react-router";

import { recordPageView } from "@/telemetry";

/**
 * Record each screen the reader lands on.
 *
 * SAFETY: only paths the router resolved are recorded. A hand-edited path
 * whose segments name no page (`/home/<anything>`) redirects before it
 * resolves, so free text never reaches the usage rows; the person segment is
 * masked by `screenPath`.
 */
export function recordScreens(router: Pick<AnyRouter, "subscribe">): void {
  let recorded: string | null = null;
  router.subscribe("onResolved", ({ toLocation }) => {
    // The path names the screen; a query change only sets it up.
    const screen = toLocation.pathname;
    if (screen === recorded) return;
    recorded = screen;
    recordPageView(screen);
  });
}
