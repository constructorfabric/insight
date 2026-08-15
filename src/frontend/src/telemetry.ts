/**
 * Usage telemetry — the page-level adoption events behind Manage → Platform
 * usage.
 *
 * Two deliberate limits. Autocapture stays off: the DOM text it collects would
 * carry person-level content out of the UI and into the event store, and only
 * page views are needed here. And a view-as session records nothing at all —
 * an operator browsing as someone else is not that person's usage, and the
 * services downstream cannot tell the two apart.
 */

import { createTelemetry, type TelemetryService } from "@gears-frontx/telemetry";

import { getUsageConfig } from "@/api/usage-client";
import type { Session } from "@/auth/types";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

const APP_NAME = "insight-frontend";

const APP_VERSION =
  (import.meta.env.VITE_APP_VERSION as string | undefined) ?? "0.0.0";

let service: TelemetryService | null = null;

/**
 * Start collecting for this session, unless the instance has usage collection
 * switched off or the session is a view-as. Resolves once that is decided.
 */
export async function startUsageTelemetry(session: Session): Promise<void> {
  if (service || session.impersonatorEmail) return;

  const config = await getUsageConfig().catch(() => null);
  if (!config?.enabled || service) return;

  service = createTelemetry({
    appName: APP_NAME,
    appVersion: APP_VERSION,
    url: `${BASE}/usage/events`,
    autocapture: false,
  })
    .identify(session.personId)
    .start();
}

/**
 * The screen a path names, with the person it is about removed: `/ic/<id>/…`
 * is one screen whoever it belongs to, and counting adoption must not become a
 * record of who read whose profile. The server does this too — this keeps the
 * id out of the request in the first place.
 */
export function screenPath(path: string): string {
  return path
    .split("/")
    .map((segment) => (isIdentifier(segment) ? ":id" : segment))
    .join("/");
}

function isIdentifier(segment: string): boolean {
  return (
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(segment) ||
    /^\d{6,}$/.test(segment)
  );
}

export function recordPageView(path: string): void {
  service?.logEvent("page_view", { path: screenPath(path) });
}

/**
 * Record anything else worth counting — a drill-down opened, an export taken.
 * `target` is what the action was aimed at, and is what the usage page ranks.
 */
export function recordUsageEvent(name: string, target?: string): void {
  service?.logEvent(name, target == null ? undefined : { target });
}

export function stopUsageTelemetry(): void {
  service?.destroy();
  service = null;
}
