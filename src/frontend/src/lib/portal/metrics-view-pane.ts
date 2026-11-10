import { useSyncExternalStore } from "react";

import { resolveZoneItem } from "@/lib/portal/nav-model";
import { usePortalItem } from "@/lib/portal/portal-nav";
import { useShellLayout } from "@/lib/portal/use-shell-layout";
import { useZoneNav } from "@/lib/portal/use-zone-nav";

let shown = false;

const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function setMetricsViewPaneShown(next: boolean): void {
  shown = next;
  for (const listener of listeners) listener();
}

export function useMetricsViewPaneShown(): boolean {
  return useSyncExternalStore(
    subscribe,
    () => shown,
    () => false,
  );
}

/**
 * The Metrics view can hand the pane's width to its table on a wide screen; the
 * shell then narrows its sidebar to the rail.
 */
export function useMetricsViewHidesPane(): boolean {
  const { activeZone } = useZoneNav();
  const item = usePortalItem();
  const shown = useMetricsViewPaneShown();
  const wide = useShellLayout() === "wide";

  const metricsViewOpen =
    activeZone === "people" && resolveZoneItem("people", item) === "metrics";
  return wide && metricsViewOpen && !shown;
}
