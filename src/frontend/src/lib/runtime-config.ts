/**
 * The per-installation runtime config the SPA reads from `/config.js`: in a
 * pod the frontend chart renders it into `window.__INSIGHT_CONFIG__`; the
 * built stub (`public/config.js`) leaves it empty everywhere else.
 *
 * Values come from operator-written YAML, so this shape is a claim, not a
 * guarantee — consumers parse the fields they read (see `parseNavHide`)
 * instead of trusting the declaration.
 */
export interface InsightRuntimeConfig {
  sentryDsn?: string;
  nav?: { hide?: readonly string[]; planned?: readonly string[] };
}

declare global {
  interface Window {
    __INSIGHT_CONFIG__?: InsightRuntimeConfig;
  }
}

export function runtimeConfig(): InsightRuntimeConfig {
  return window.__INSIGHT_CONFIG__ ?? {};
}
