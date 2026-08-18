import type { RouterHistory } from "@tanstack/react-router";

import { validatePortalSearch } from "@/lib/portal/portal-search";
import { recordPageView } from "@/telemetry";

// INVARIANT: portal screens differ only by the zone/item search params, so
// only a history subscription sees them change.
export function recordScreens(history: RouterHistory): void {
  let recorded: string | null = null;
  const record = () => {
    const screen = currentScreen();
    if (screen === recorded) return;
    recorded = screen;
    recordPageView(screen);
  };
  history.subscribe(record);
  record();
}

function currentScreen(): string {
  const { zone, item } = validatePortalSearch(
    Object.fromEntries(new URLSearchParams(window.location.search)),
  );
  const parts = [zone, item].filter((value): value is string => Boolean(value));
  return parts.length
    ? `${window.location.pathname}/${parts.join("/")}`
    : window.location.pathname;
}
