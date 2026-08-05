import { authStore } from "./auth-store";
import type { AuthStatus } from "./types";

/** A server unix-seconds timestamp: finite positive number, else 0 (absent). */
function unixSeconds(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : 0;
}

/**
 * Probe `GET /auth/me` once and populate the store. The browser sends the
 * `__Host-sid` cookie (same-origin, credentials included); the authenticator
 * returns the session summary or 401. Any non-200 fails closed to
 * `unauthenticated` so the app redirects to login rather than rendering
 * half-authenticated. Called once at boot (main.tsx) before the router mounts.
 */
export async function loadSession(): Promise<AuthStatus> {
  try {
    const res = await fetch("/auth/me", {
      credentials: "include",
      headers: { Accept: "application/json" },
    });
    if (!res.ok) {
      authStore.setUnauthenticated();
      return "unauthenticated";
    }
    const body = (await res.json()) as {
      user?: string;
      email?: string;
      tenant_id?: string;
      roles?: string[];
      csrf_token?: string;
      expires_at?: number;
      refresh_at?: number;
      impersonator_email?: string;
    };
    // Fail closed on a missing CSRF token. A live session always carries one
    // (the authenticator echoes it on /auth/me), and it is required to send
    // `X-CSRF-Token` on state-changing /auth/* (logout, refresh). Storing an
    // empty token would let those calls omit the header and be rejected
    // server-side. An absent token here means a bug or a not-yet-CSRF-aware
    // backend — treat it as no session (redirects to login, which re-mints a
    // session with a token). Requires the backend to be deployed first.
    if (!body.csrf_token) {
      console.warn(
        "/auth/me returned no csrf_token; treating as unauthenticated"
      );
      authStore.setUnauthenticated();
      return "unauthenticated";
    }
    authStore.setAuthenticated({
      personId: body.user ?? "",
      email: body.email ?? "",
      tenantId: body.tenant_id ?? "",
      roles: body.roles ?? [],
      csrfToken: body.csrf_token,
      // Missing or malformed timestamps become 0, which the refresh driver
      // treats as "never schedule" — the session then just times out as it
      // did before the driver existed. The JSON cast above is compile-time
      // only, so guard the wire values like refresh.ts guards its inputs.
      expiresAt: unixSeconds(body.expires_at),
      refreshAt: unixSeconds(body.refresh_at),
      // Present only on `__override` view-as sessions (insight#1941).
      impersonatorEmail: body.impersonator_email || null,
    });
    return "authenticated";
  } catch {
    // Network error reaching the authenticator — fail closed.
    authStore.setUnauthenticated();
    return "unauthenticated";
  }
}
