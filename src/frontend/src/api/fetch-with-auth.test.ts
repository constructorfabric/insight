import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { authStore } from "@/auth/auth-store";
import { fetchWithAuth, resetSessionProbe } from "./fetch-with-auth";
import { makeSession } from "@/test/session";

const fetchMock = () => globalThis.fetch as ReturnType<typeof vi.fn>;

function initOfLastCall(): RequestInit {
  const calls = fetchMock().mock.calls;
  return (calls[calls.length - 1]?.[1] ?? {}) as RequestInit;
}

describe("fetchWithAuth", () => {
  let assign: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    resetSessionProbe();
    // signIn() latches `redirecting` for the page's lifetime, and the module
    // outlives a test. A bfcache restore is the seam it offers to clear it —
    // without this, only the first test in the file can observe a bounce.
    const restored = new Event("pageshow");
    Object.defineProperty(restored, "persisted", { value: true });
    window.dispatchEvent(restored);
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

  it("bounces into the login flow when the authenticator confirms the session ended", async () => {
    // The mock answers the /auth/me probe with 401 too — a session really gone.
    const r401 = new Response(null, { status: 401 });
    fetchMock().mockResolvedValue(r401);
    const res = await fetchWithAuth("/x");
    expect(res).toBe(r401);
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign.mock.calls[0][0]).toMatch(/^\/auth\/login\?return_to=/);
  });

  /** `/auth/me` as the authenticator answers it for a session that stands. */
  function liveSession(): Response {
    return Response.json({
      user: "p-1",
      email: "bob.park@example.com",
      tenant_id: "t-1",
      roles: ["user"],
      csrf_token: "csrf-1",
      expires_at: 4102444800,
      refresh_at: 4102444710,
    });
  }

  it("keeps a session the authenticator still vouches for when an endpoint 401s", async () => {
    const r401 = new Response(null, { status: 401 });
    fetchMock().mockImplementation((input: RequestInfo | URL) =>
      Promise.resolve(String(input) === "/auth/me" ? liveSession() : r401)
    );

    const res = await fetchWithAuth("/api/v3/v1/datasets/");

    expect(res).toBe(r401);
    expect(authStore.getSnapshot().status).toBe("authenticated");
    expect(assign).not.toHaveBeenCalled();
  });

  it("probes the authenticator once for 401s that land together", async () => {
    const r401 = new Response(null, { status: 401 });
    fetchMock().mockImplementation((input: RequestInfo | URL) =>
      Promise.resolve(String(input) === "/auth/me" ? liveSession() : r401)
    );

    await Promise.all([
      fetchWithAuth("/a"),
      fetchWithAuth("/b"),
      fetchWithAuth("/c"),
    ]);

    const probes = fetchMock().mock.calls.filter(
      (call) => String(call[0]) === "/auth/me"
    );
    expect(probes).toHaveLength(1);
  });

  it("bounces when the authenticator cannot be reached at all", async () => {
    const r401 = new Response(null, { status: 401 });
    fetchMock().mockImplementation((input: RequestInfo | URL) =>
      String(input) === "/auth/me"
        ? Promise.reject(new Error("network down"))
        : Promise.resolve(r401)
    );

    await fetchWithAuth("/x");

    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(assign).toHaveBeenCalledTimes(1);
  });

  it("probes again once the confirmation has aged out", async () => {
    vi.useFakeTimers();
    const r401 = new Response(null, { status: 401 });
    fetchMock().mockImplementation((input: RequestInfo | URL) =>
      Promise.resolve(String(input) === "/auth/me" ? liveSession() : r401)
    );

    await fetchWithAuth("/a");
    await fetchWithAuth("/b"); // inside the window: rides the confirmation
    vi.setSystemTime(Date.now() + 6_000);
    await fetchWithAuth("/c");

    const probes = fetchMock().mock.calls.filter(
      (call) => String(call[0]) === "/auth/me"
    );
    expect(probes).toHaveLength(2);
    expect(assign).not.toHaveBeenCalled();
    vi.useRealTimers();
  });
});
