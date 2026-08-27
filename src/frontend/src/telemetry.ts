import { createTelemetry, type TelemetryService } from "@gears-frontx/telemetry";

import { getUsageConfig } from "@/api/usage-client";
import type { Session } from "@/auth/types";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export const APP_NAME = "insight-frontend";

export const APP_VERSION =
  (import.meta.env.VITE_APP_RELEASE as string | undefined) || "0.0.0";

let service: TelemetryService | null = null;

const pending: Array<{ name: string; data: Record<string, unknown> }> = [];
const MAX_PENDING = 50;

function emit(name: string, data: Record<string, unknown>): void {
  if (service) {
    service.logEvent(name, data);
    return;
  }
  if (pending.length < MAX_PENDING) pending.push({ name, data });
}

export async function startUsageTelemetry(session: Session): Promise<void> {
  if (service || session.impersonatorEmail) return;

  const config = await getUsageConfig().catch(() => null);
  if (!config?.enabled) {
    pending.length = 0;
    return;
  }

  service = createTelemetry({
    appName: APP_NAME,
    appVersion: APP_VERSION,
    url: `${BASE}/usage/events`,
    apiVersion: 2,
    autocapture: false,
    // The SDK keeps its session in origin-wide storage, so two people signing
    // in on one browser would share a session id and merge into one visit.
    storagePrefix: `insight-usage:${session.personId}`,
  })
    .identify(session.personId)
    .start();

  for (const event of pending.splice(0)) {
    service.logEvent(event.name, event.data);
  }
}

/** Adoption counting must not become a record of who read whose profile. */
export function screenPath(path: string): string {
  const segments = path.split("/");
  return segments
    .map((segment, i) =>
      // The segment after `/ic` is a person key whatever shape it arrives in;
      // matching on shape alone lets an email through.
      segments[i - 1] === "ic" || isIdentifier(segment) ? ":id" : segment,
    )
    .join("/");
}

function isIdentifier(segment: string): boolean {
  return (
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(segment) ||
    /^\d{6,}$/.test(segment)
  );
}

let lastPath = "";

export function recordPageView(path: string): void {
  lastPath = screenPath(path);
  emit("page_view", { path: lastPath });
}

/** The shape of a scope, never the person it is rooted at. */
export function scopeLabel(scope: {
  root: string | null;
  directOnly: boolean;
  attrFilter?: { key: string; value: string };
}): string {
  if (scope.attrFilter) return `attr:${scope.attrFilter.key}`;
  if (!scope.root) return "whole-org";
  return scope.directOnly ? "subtree-direct" : "subtree";
}

export function recordUsageEvent(name: string, target: string): void {
  emit(name, { target, path: lastPath });
}

/** The screen the reader is on, named as the usage rows name it. */
export function currentScreen(): string {
  return lastPath;
}
