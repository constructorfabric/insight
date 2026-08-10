import * as Sentry from "@sentry/react";

const LOCAL_HOSTNAMES = new Set(["localhost", "127.0.0.1", "[::1]"]);

export function initSentry(router: unknown): void {
  const dsn = import.meta.env.VITE_SENTRY_DSN;
  if (!dsn) return;

  Sentry.init({
    dsn,
    environment: resolveEnvironment(),
    release: import.meta.env.VITE_APP_RELEASE || undefined,
    // Route pattern rather than resolved URL, so ids stay out of event names.
    integrations: [Sentry.tanstackRouterBrowserTracingIntegration(router)],
    tracesSampleRate: 0.1,
  });
}

/** One image serves every stand, so the hostname is the only label it has. */
function resolveEnvironment(): string {
  const { hostname } = window.location;
  return LOCAL_HOSTNAMES.has(hostname) ? "local" : hostname;
}
