// A failed OIDC callback bounces the browser back to the SPA with
// `?auth_error=<reason>` (insight#2032) — there is no page loaded at
// `/auth/callback`, so the authenticator redirects instead of answering
// problem+json the user cannot act on. The retryable reasons (an expired
// login state after a slow IdP round-trip, a replayed callback, an IdP
// hiccup) are fixed by simply logging in again, so boot restarts the flow
// once; a sessionStorage attempt counter halts a persistent failure on the
// error screen instead of looping browser -> IdP forever. `access_denied`
// (unknown person / no tenant) never auto-retries — a silent SSO hop would
// just reproduce it.

const ATTEMPTS_KEY = "insight.auth-error-attempts";
const MAX_AUTO_RETRIES = 1;

export type AuthError = {
  /** The authenticator's fixed reason code, e.g. `state_expired`. */
  code: string;
  /** Restart the login automatically, or halt on the error screen. */
  autoRetry: boolean;
};

/**
 * If the current URL carries `auth_error`, strip it (so reloads, copied
 * links, and the next login's `return_to` don't carry it), count the attempt,
 * and return the code plus the auto-retry verdict. `null` when absent.
 */
export function consumeAuthErrorParam(): AuthError | null {
  const url = new URL(window.location.href);
  if (!url.searchParams.has("auth_error")) return null;
  const code = url.searchParams.get("auth_error") ?? "";
  url.searchParams.delete("auth_error");
  window.history.replaceState(null, "", url.pathname + url.search + url.hash);
  // An empty value (hand-crafted URL — the authenticator always sends a
  // reason) is stripped but neither counted nor acted on.
  if (!code) return null;
  const attempts = readAttempts() + 1;
  // An unpersisted attempt must not auto-retry: the next bounce would read
  // zero again and the guard would never trip.
  const counted = writeAttempts(attempts);
  return {
    code,
    autoRetry:
      counted && code !== "access_denied" && attempts <= MAX_AUTO_RETRIES,
  };
}

/** Reset the counter: on a confirmed session, or on a user-driven retry. */
export function clearAuthErrorAttempts(): void {
  try {
    sessionStorage.removeItem(ATTEMPTS_KEY);
  } catch {
    // Storage unavailable — nothing to clear.
  }
}

function readAttempts(): number {
  try {
    const parsed = Number(sessionStorage.getItem(ATTEMPTS_KEY) ?? "0");
    return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
  } catch {
    // Storage unavailable: attempts cannot be counted across the login
    // round-trips, so fail closed to the error screen rather than risk an
    // uncounted redirect loop.
    return MAX_AUTO_RETRIES;
  }
}

function writeAttempts(attempts: number): boolean {
  try {
    sessionStorage.setItem(ATTEMPTS_KEY, String(attempts));
    return true;
  } catch {
    return false;
  }
}
