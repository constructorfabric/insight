import type { RouterHistory } from "@tanstack/react-router";

import { validatePortalSearch } from "@/lib/portal/portal-search";
import { recordPageView } from "@/telemetry";

// INVARIANT: portal screens differ only by the zone/item search params, so
// only a history subscription sees them change.
export function recordScreens(history: RouterHistory): void {
  let recorded: string | null = null;
  const record = () => {
    // The router defers its pushState to a microtask, so `window` still holds
    // the previous screen here.
    const screen = screenOf(history.location);
    if (screen === recorded) return;
    recorded = screen;
    recordPageView(screen);
  };
  history.subscribe(record);
  record();
}

function screenOf(location: { pathname: string; search: string }): string {
  const { zone, item } = validatePortalSearch(
    Object.fromEntries(new URLSearchParams(location.search)),
  );
  const parts = [zone, item].filter((value): value is string => Boolean(value));
  return parts.length
    ? `${location.pathname}/${parts.join("/")}`
    : location.pathname;
}
