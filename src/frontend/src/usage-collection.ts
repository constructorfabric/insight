import type { RouterHistory } from "@tanstack/react-router";

import { recordPageView } from "@/telemetry";

// The path names the screen; the query only sets it up, so a filter change is
// not a new screen.
export function recordScreens(history: RouterHistory): void {
  let recorded: string | null = null;
  const record = () => {
    // WORKAROUND: the router defers its pushState to a microtask, so `window`
    // still holds the previous screen here.
    const screen = history.location.pathname;
    if (screen === recorded) return;
    recorded = screen;
    recordPageView(screen);
  };
  history.subscribe(record);
  record();
}
