import type { Dashboard, DashboardItem } from "@/api/custom-client";

/**
 * What a dashboard draws, however its body says it.
 *
 * `widgets: ["a", "b"]` is the older shorthand for a list of nothing but
 * widgets, and every board written before there was anything else to put in
 * one says it that way.
 *
 * Lives here rather than beside the client: a test that mocks the client
 * module would stub this too, and the page would draw nothing.
 */
export function dashboardItems(dashboard: Dashboard): DashboardItem[] {
  return (
    dashboard.items ?? (dashboard.widgets ?? []).map((widget) => ({ widget }))
  );
}
