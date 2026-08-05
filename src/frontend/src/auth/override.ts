// The `?__override=<email>` view-as entry point (insight#1941). The historical
// portal URL — https://<host>/?__override=<email> — must keep working, so the
// SPA consumes the parameter at boot and bounces the whole page into the login
// flow carrying it. The authenticator honors it only when its
// `override_enabled` flag is on (dev/demo environments); everywhere else the
// login proceeds as the caller.

/**
 * If the current URL carries `__override`, redirect into
 * `/auth/login?__override=...&return_to=<current URL without the parameter>`
 * and return `true` — the caller must then skip mounting the app (the page is
 * navigating away). Works while already logged in: the authenticator's
 * session-fixation guard revokes the presented session at the callback, and
 * the IdP hop is silent SSO.
 */
export function consumeOverrideParam(): boolean {
  const url = new URL(window.location.href);
  const target = url.searchParams.get("__override");
  if (!target) return false;
  url.searchParams.delete("__override");
  const returnTo = url.pathname + url.search;
  window.location.assign(
    `/auth/login?__override=${encodeURIComponent(target)}&return_to=${encodeURIComponent(returnTo)}`
  );
  return true;
}
