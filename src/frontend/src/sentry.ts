import * as Sentry from "@sentry/react";

import { runtimeConfig } from "@/lib/runtime-config";

const LOCAL_HOSTNAMES = new Set(["localhost", "127.0.0.1", "[::1]"]);

export function initSentry(router: unknown): void {
  // Nullish, not `||`: the chart renders "" to mean reporting off, and that
  // has to outrank a build-time DSN rather than fall through to it.
  const dsn = runtimeConfig().sentryDsn ?? import.meta.env.VITE_SENTRY_DSN;
  if (!dsn) return;

  // The DSN arrives from the deploy, so a malformed one is a config mistake,
  // not a code path — it must not take the app down on boot.
  try {
    Sentry.init({
      dsn,
      environment: resolveEnvironment(),
      release: import.meta.env.VITE_APP_RELEASE || undefined,
      // Route pattern rather than resolved URL, so ids stay out of event names.
      integrations: [Sentry.tanstackRouterBrowserTracingIntegration(router)],
      tracesSampleRate: 0.1,
    });
  } catch (error) {
    console.error("Sentry init failed", error);
  }
}

/** One image serves every stand, so the hostname is the only label it has. */
function resolveEnvironment(): string {
  const { hostname } = window.location;
  return LOCAL_HOSTNAMES.has(hostname) ? "local" : hostname;
}
