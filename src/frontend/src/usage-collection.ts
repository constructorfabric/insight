import type { RouterHistory } from "@tanstack/react-router";

import { GROUPS } from "@/lib/insight/groups";
import { resolveMode } from "@/lib/portal/identity-modes";
import {
  DIRECTIONS,
  ZONES,
  lensSlug,
  zoneById,
  zoneItems,
} from "@/lib/portal/nav-model";
import { type PortalSearch, validatePortalSearch } from "@/lib/portal/portal-search";
import { recordPageView } from "@/telemetry";

// INVARIANT: portal screens differ only by search params — zone, item, and the
// per-zone detail below — so only a history subscription sees them change.
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
  const search = validatePortalSearch(
    Object.fromEntries(new URLSearchParams(location.search)),
  );
  const zone = zoneById(search.zone ?? null)?.id;
  const item = search.item && SCREEN_ITEMS.has(search.item) ? search.item : undefined;
  const parts = [zone, item, ...detailOf(zone, item, search)].filter(
    (value): value is string => Boolean(value),
  );
  return parts.length
    ? `${location.pathname}/${parts.join("/")}`
    : location.pathname;
}

// A zone change does not clear `dir`/`lens`/`mode`, so read each only under the
// zone that renders it.
function detailOf(
  zone: string | undefined,
  item: string | undefined,
  search: PortalSearch,
): readonly string[] {
  if (zone === "directions") {
    const direction = DIRECTIONS.find((d) => d.id === search.dir);
    if (!direction) return [];
    const lens = search.lens;
    return lens && direction.lenses.includes(lens)
      ? [direction.id, lensSlug(lens)]
      : [direction.id];
  }
  if (zone === "manage" && item === "identities" && search.mode) {
    return [resolveMode(search.mode)];
  }
  return [];
}
