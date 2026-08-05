import { authStore } from "@/auth/auth-store";
import { signIn } from "@/auth/use-auth";

/**
 * Fetch an in-cluster API through the gateway. Auth is entirely cookie-based
 * (NGINX_BFF): we send the `__Host-sid` cookie via `credentials: "include"` and
 * the gateway injects the ES256 gateway JWT downstream — the SPA attaches no
 * `Authorization` header and asserts no tenant. A 401 means the session is gone
 * or expired; bounce the whole page into the login flow (there is no
 * client-side token to refresh).
 */
export async function fetchWithAuth(
  input: RequestInfo | URL,
  init: RequestInit = {},
): Promise<Response> {
  const res = await fetch(input, { ...init, credentials: "include" });

  if (res.status === 401) {
    authStore.setUnauthenticated();
    signIn();
  }
  return res;
}
