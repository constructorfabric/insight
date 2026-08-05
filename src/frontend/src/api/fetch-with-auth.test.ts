import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { authStore } from "@/auth/auth-store";
import { fetchWithAuth } from "./fetch-with-auth";
import { makeSession } from "@/test/session";

const fetchMock = () => globalThis.fetch as ReturnType<typeof vi.fn>;

function initOfLastCall(): RequestInit {
  const calls = fetchMock().mock.calls;
  return (calls[calls.length - 1]?.[1] ?? {}) as RequestInit;
}

describe("fetchWithAuth", () => {
  let assign: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    authStore.reset();
    authStore.setAuthenticated(makeSession());
    vi.stubGlobal("fetch", vi.fn());
    // jsdom's `window.location.assign` is a non-configurable no-op; replace
    // the whole location object so the full-page login bounce is observable.
    assign = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, assign, pathname: "/dash", search: "?q=1" },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    authStore.reset();
  });

  it("sends credentials:'include' and no Authorization / X-Tenant-ID header", async () => {
    fetchMock().mockResolvedValue(new Response(null, { status: 200 }));
    await fetchWithAuth("/x", { headers: { Accept: "application/json" } });
    const init = initOfLastCall();
    expect(init.credentials).toBe("include");
    const headers = new Headers(init.headers);
    expect(headers.get("Authorization")).toBeNull();
    expect(headers.get("X-Tenant-ID")).toBeNull();
  });

  it("returns non-401 responses unchanged without touching auth", async () => {
    const ok = new Response(null, { status: 200 });
    fetchMock().mockResolvedValue(ok);
    const res = await fetchWithAuth("/x");
    expect(res).toBe(ok);
    expect(authStore.getSnapshot().status).toBe("authenticated");
    expect(assign).not.toHaveBeenCalled();
  });

  it("on a 401 marks the session unauthenticated and bounces into the login flow", async () => {
    const r401 = new Response(null, { status: 401 });
    fetchMock().mockResolvedValue(r401);
    const res = await fetchWithAuth("/x");
    expect(res).toBe(r401);
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign.mock.calls[0][0]).toMatch(/^\/auth\/login\?return_to=/);
  });
});
