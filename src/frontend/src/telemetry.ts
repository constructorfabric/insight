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

export function recordPageView(path: string): void {
  service?.logEvent("page_view", { path });
}

export function stopUsageTelemetry(): void {
  service?.destroy();
  service = null;
}
