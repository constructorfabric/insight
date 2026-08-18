import type { RouterHistory } from "@tanstack/react-router";

import { GROUPS } from "@/lib/insight/groups";
import { ZONES, zoneById, zoneItems } from "@/lib/portal/nav-model";
import { validatePortalSearch } from "@/lib/portal/portal-search";
import { recordPageView } from "@/telemetry";

// INVARIANT: portal screens differ only by the zone/item search params, so
// only a history subscription sees them change.
export function recordScreens(history: RouterHistory): void {
  let recorded: string | null = null;
  const record = () => {
    // WORKAROUND: the router defers its pushState to a microtask, so `window`
    // still holds the previous screen here.
    const screen = screenOf(history.location);
    if (screen === recorded) return;
    recorded = screen;
    recordPageView(screen);
  };
  history.subscribe(record);
  record();
}

const SCREEN_ITEMS = new Set([
  ...ZONES.flatMap((zone) => zoneItems(zone.id).map((item) => item.id)),
  ...GROUPS.map((group) => group.id),
]);

// SAFETY: `zone` and `item` are free text from the URL, and `screenPath`
// redacts only identifier-shaped segments — an email would be recorded as-is.
function screenOf(location: { pathname: string; search: string }): string {
  const { zone, item } = validatePortalSearch(
    Object.fromEntries(new URLSearchParams(location.search)),
  );
  const parts = [
    zoneById(zone ?? null)?.id,
    item && SCREEN_ITEMS.has(item) ? item : undefined,
  ].filter((value): value is string => Boolean(value));
  return parts.length
    ? `${location.pathname}/${parts.join("/")}`
    : location.pathname;
}
