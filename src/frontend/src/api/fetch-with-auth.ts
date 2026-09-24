import { authStore } from "@/auth/auth-store";
import { loadSession } from "@/auth/session";
import { signIn } from "@/auth/use-auth";

/** One probe answers every 401 in flight; cleared once it settles. */
let sessionProbe: Promise<void> | null = null;
/** When the last probe confirmed the session was live (epoch ms, 0 = never). */
let confirmedAt = 0;
/** How long a confirmation stands before another 401 is worth re-probing.
 *  Without it a retried query, or a page of widgets all hitting one bad path,
 *  probes once per 401 instead of once per incident. */
const CONFIRMED_FOR_MS = 5_000;

/** Drops the in-flight probe and the confirmation window. Tests only — this
 *  module's state outlives a single test the way it outlives a page view. */
export function resetSessionProbe(): void {
  sessionProbe = null;
  confirmedAt = 0;
}

/**
 * A 401 is not proof the session ended. The services answer 401 for every path
 * they do not route — a name that reached the client empty, a stale build
 * asking for a withdrawn endpoint — so treating one as a dead session signs a
 * working reader out and reloads the page under them. Ask the authenticator
 * which of the two it is, and bounce only when the session is really gone.
 */
async function bounceIfSessionEnded(): Promise<void> {
  // Already failed closed — whatever did that owns the redirect; re-probing
  // could only answer 200 and repaint the app under a navigation in flight.
  if (authStore.getSnapshot().status === "unauthenticated") {
    signIn();
    return;
  }
  if (Date.now() - confirmedAt < CONFIRMED_FOR_MS) return;

  const mine = (sessionProbe ??= loadSession()
    .then((status) => {
      // loadSession() has already failed the store closed by this point, and
      // bounds its own request — a hung authenticator lands here as closed.
      if (status === "unauthenticated") signIn();
      else confirmedAt = Date.now();
    })
    .catch(() => {
      // signIn() can throw in a sandboxed frame; the caller still gets its
      // Response rather than this rejection.
    })
    .finally(() => {
      if (sessionProbe === mine) sessionProbe = null;
    }));

  await mine;
}

/**
 * Fetch an in-cluster API through the gateway. Auth is entirely cookie-based
 * (NGINX_BFF): we send the `__Host-sid` cookie via `credentials: "include"` and
 * the gateway injects the ES256 gateway JWT downstream — the SPA attaches no
 * `Authorization` header and asserts no tenant. A 401 is checked against
 * `/auth/me` before it is allowed to end the session (there is no client-side
 * token to refresh); the response itself is handed back either way, so a
 * caller that can say something about its own 401 still gets the chance.
 */
export async function fetchWithAuth(
  input: RequestInfo | URL,
  init: RequestInit = {},
): Promise<Response> {
  const res = await fetch(input, { ...init, credentials: "include" });

  if (res.status === 401) await bounceIfSessionEnded();
  return res;
}
