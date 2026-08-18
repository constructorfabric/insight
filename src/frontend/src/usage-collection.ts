import type { RouterHistory } from "@tanstack/react-router";

import { validatePortalSearch } from "@/lib/portal/portal-search";
import { recordPageView } from "@/telemetry";

/**
 * Record every screen a reader opens, the one they land on included.
 *
 * INVARIANT: portal screens differ only by the zone/item search params, so
 * only a history subscription sees them change.
 */
export function recordScreens(history: RouterHistory): () => void {
  let recorded: string | null = null;
  const record = () => {
    // `history.location`, never `window.location`: the router queues its
    // pushState in a microtask and notifies subscribers BEFORE that lands, so
    // window still holds the screen the reader left — which recorded every
    // visit one hop late and never recorded the last screen of a session.
    const screen = screenOf(history.location);
    if (screen === recorded) return;
    recorded = screen;
    recordPageView(screen);
  };
  const stop = history.subscribe(record);
  record();
  return stop;
}

function screenOf(location: { pathname: string; search: string }): string {
  const { zone, item } = validatePortalSearch(
    Object.fromEntries(new URLSearchParams(location.search)),
  );
  const parts = [zone, item].filter((value): value is string => Boolean(value));
  return parts.length ? `${location.pathname}/${parts.join("/")}` : location.pathname;
}
