import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { authStore } from "./auth-store";
import { startSessionRefresh } from "./refresh";
import { makeSession } from "@/test/session";

// Coordination keys — mirror refresh.ts.
const LEASE_KEY = "insight.auth.refresh.leader";
const FALLBACK_MSG_KEY = "insight.auth.refresh.msg";

// Frozen "now" for every test (unix seconds / ms).
const T0 = 1770000000;

// Server responses deliberately use an *asymmetric* refresh_at↔expires_at gap
// (83 s, not the backend's default 90 s margin): an implementation that
// client-computes refresh_at from expires_at would fail these expectations.
const REFRESH_OK_1 = { expires_at: T0 + 700, refresh_at: T0 + 617 };
const REFRESH_OK_2 = { expires_at: T0 + 1300, refresh_at: T0 + 1217 };

const fetchMock = () => globalThis.fetch as ReturnType<typeof vi.fn>;

function jsonResponse(body: unknown, status = 200, headers?: Record<string, string>): Response {
  return new Response(JSON.stringify(body), { status, headers });
}

function session(overrides: { refreshAt?: number; expiresAt?: number } = {}) {
  return makeSession({
    refreshAt: overrides.refreshAt ?? T0 + 100,
    expiresAt: overrides.expiresAt ?? T0 + 205,
  });
}

describe("session refresh driver", () => {
  let assign: ReturnType<typeof vi.fn>;
  let stop: (() => void) | null = null;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(T0 * 1000);
    localStorage.clear();
    authStore.reset();
    vi.stubGlobal("fetch", vi.fn());
    // Force the localStorage fan-out path by default; the BroadcastChannel
    // path gets its own test with an observable fake.
    vi.stubGlobal("BroadcastChannel", undefined);
    assign = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, assign, pathname: "/dash", search: "" },
    });
  });

  afterEach(() => {
    stop?.();
    stop = null;
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    authStore.reset();
    localStorage.clear();
  });

  it("POSTs /auth/refresh at refresh_at with the CSRF header, then reschedules from the server's refresh_at", async () => {
    authStore.setAuthenticated(session());
    fetchMock().mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(99_000);
    expect(fetchMock()).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock().mock.calls[0];
    expect(url).toBe("/auth/refresh");
    expect(init).toMatchObject({ method: "POST", credentials: "include" });
    expect(init.headers).toMatchObject({ "X-CSRF-Token": "csrf-1" });

    // The store carries the fresh server-supplied schedule…
    expect(authStore.getSnapshot().session).toMatchObject({
      expiresAt: T0 + 700,
      refreshAt: T0 + 617,
    });
    // …and identity fields survive the fold-in.
    expect(authStore.getSnapshot().session?.csrfToken).toBe("csrf-1");

    // Next refresh fires at the *new* refresh_at (T0+617), not a client guess.
    fetchMock().mockResolvedValueOnce(jsonResponse(REFRESH_OK_2));
    await vi.advanceTimersByTimeAsync(516_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2);
  });

  it("refreshes immediately when refresh_at is already in the past", async () => {
    authStore.setAuthenticated(session({ refreshAt: T0 - 5 }));
    fetchMock().mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock()).toHaveBeenCalledTimes(1);
  });

  it("never schedules when the backend sent no refresh_at (stored as 0)", async () => {
    authStore.setAuthenticated(session({ refreshAt: 0 }));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(3_600_000);
    expect(fetchMock()).not.toHaveBeenCalled();
  });

  it("on 401 clears the session, publishes the expiry, and bounces into login", async () => {
    authStore.setAuthenticated(session());
    fetchMock().mockResolvedValueOnce(new Response(null, { status: 401 }));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);

    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(assign).toHaveBeenCalledTimes(1);
    expect(assign.mock.calls[0][0]).toMatch(/^\/auth\/login\?return_to=/);
    // Fallback fan-out so other tabs stop trusting the session too.
    expect(localStorage.getItem(FALLBACK_MSG_KEY)).toContain('"expired"');
  });

  it("on 429 retries after the Retry-After hint", async () => {
    authStore.setAuthenticated(session());
    fetchMock()
      .mockResolvedValueOnce(jsonResponse({}, 429, { "Retry-After": "7" }))
      .mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(6_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2);
    expect(authStore.getSnapshot().session?.refreshAt).toBe(T0 + 617);
  });

  it("on a network error retries and recovers", async () => {
    authStore.setAuthenticated(session());
    fetchMock()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);
    expect(authStore.getSnapshot().status).toBe("authenticated");

    // Retry cadence for transient failures is 15 s.
    await vi.advanceTimersByTimeAsync(15_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2);
    expect(authStore.getSnapshot().session?.refreshAt).toBe(T0 + 617);
  });

  it("on 403 re-primes the CSRF token and schedule via /auth/me", async () => {
    authStore.setAuthenticated(session());
    fetchMock()
      .mockResolvedValueOnce(new Response(null, { status: 403 }))
      .mockResolvedValueOnce(
        jsonResponse({ user: "p-1", csrf_token: "csrf-2", ...REFRESH_OK_1 }),
      );
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2);
    expect(fetchMock().mock.calls[1][0]).toBe("/auth/me");
    expect(authStore.getSnapshot().session).toMatchObject({
      csrfToken: "csrf-2",
      refreshAt: T0 + 617,
    });

    // The re-primed schedule is armed with the *new* token.
    fetchMock().mockResolvedValueOnce(jsonResponse(REFRESH_OK_2));
    await vi.advanceTimersByTimeAsync(517_000);
    expect(fetchMock()).toHaveBeenCalledTimes(3);
    expect(fetchMock().mock.calls[2][1].headers).toMatchObject({ "X-CSRF-Token": "csrf-2" });
  });

  it("floors the retry when a persistent 403 re-primes an already-due refresh_at", async () => {
    authStore.setAuthenticated(session());
    fetchMock()
      .mockResolvedValueOnce(new Response(null, { status: 403 }))
      // /auth/me echoes a refresh_at that is already in the past — without a
      // floor the driver would spin refresh+me in a zero-delay loop.
      .mockResolvedValueOnce(
        jsonResponse({ user: "p-1", csrf_token: "csrf-2", expires_at: T0 + 180, refresh_at: T0 + 99 }),
      )
      .mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2); // 403 + /auth/me, no instant re-POST

    await vi.advanceTimersByTimeAsync(14_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1_000); // RETRY_MS floor
    expect(fetchMock()).toHaveBeenCalledTimes(3);
    expect(fetchMock().mock.calls[2][0]).toBe("/auth/refresh");
  });

  it("clears the session when the 403 re-probe finds it dead", async () => {
    authStore.setAuthenticated(session());
    fetchMock()
      .mockResolvedValueOnce(new Response(null, { status: 403 }))
      .mockResolvedValueOnce(new Response(null, { status: 401 })); // /auth/me
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);

    // (signIn's one observable bounce per file is spent by the 401 test;
    // the shared expire() path is asserted there.)
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    // Other tabs learn of the expiry like on the 401 path — via the
    // localStorage transport, since stop() already closed the channel.
    expect(localStorage.getItem(FALLBACK_MSG_KEY)).toContain('"expired"');
    await vi.advanceTimersByTimeAsync(3_600_000);
    expect(fetchMock()).toHaveBeenCalledTimes(2); // driver stopped, no loop
  });

  it("stays quiet while another tab holds a fresh lease, then takes over once it goes stale", async () => {
    localStorage.setItem(LEASE_KEY, JSON.stringify({ id: "other-tab", ts: T0 * 1000 }));
    authStore.setAuthenticated(session({ refreshAt: T0 + 2 }));
    fetchMock().mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    // Refresh comes due, but the other tab owns it.
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fetchMock()).not.toHaveBeenCalled();

    // The other tab never renews (closed/crashed): once the lease is stale a
    // heartbeat steals it and fires the overdue refresh immediately.
    await vi.advanceTimersByTimeAsync(11_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);
    const lease = JSON.parse(localStorage.getItem(LEASE_KEY) ?? "{}") as { id?: string };
    expect(lease.id).toBeDefined();
    expect(lease.id).not.toBe("other-tab");
  });

  it("a follower applies a 'refreshed' fan-out message instead of refreshing itself", () => {
    localStorage.setItem(LEASE_KEY, JSON.stringify({ id: "other-tab", ts: T0 * 1000 }));
    authStore.setAuthenticated(session());
    stop = startSessionRefresh();

    window.dispatchEvent(
      new StorageEvent("storage", {
        key: FALLBACK_MSG_KEY,
        newValue: JSON.stringify({ kind: "refreshed", expiresAt: T0 + 700, refreshAt: T0 + 617 }),
      }),
    );

    expect(authStore.getSnapshot().session).toMatchObject({
      expiresAt: T0 + 700,
      refreshAt: T0 + 617,
    });
    expect(fetchMock()).not.toHaveBeenCalled();
  });

  it("a follower clears the session on an 'expired' fan-out message", () => {
    localStorage.setItem(LEASE_KEY, JSON.stringify({ id: "other-tab", ts: T0 * 1000 }));
    authStore.setAuthenticated(session());
    stop = startSessionRefresh();

    window.dispatchEvent(
      new StorageEvent("storage", {
        key: FALLBACK_MSG_KEY,
        newValue: JSON.stringify({ kind: "expired" }),
      }),
    );

    expect(authStore.getSnapshot().status).toBe("unauthenticated");
  });

  it("ignores unknown and malformed fan-out messages", () => {
    localStorage.setItem(LEASE_KEY, JSON.stringify({ id: "other-tab", ts: T0 * 1000 }));
    authStore.setAuthenticated(session());
    stop = startSessionRefresh();

    for (const newValue of [
      JSON.stringify({ kind: "some-v2-message" }), // newer deploy in another tab
      JSON.stringify({ kind: "refreshed", expiresAt: "soon", refreshAt: null }),
      JSON.stringify(42),
      "not json at all",
    ]) {
      window.dispatchEvent(new StorageEvent("storage", { key: FALLBACK_MSG_KEY, newValue }));
    }

    expect(authStore.getSnapshot().status).toBe("authenticated");
    expect(authStore.getSnapshot().session?.refreshAt).toBe(T0 + 100);
  });

  it("discards a refresh result that lands after signOut cleared the store", async () => {
    authStore.setAuthenticated(session());
    let resolveFetch: (res: Response) => void = () => {};
    fetchMock().mockReturnValueOnce(
      new Promise<Response>((resolve) => {
        resolveFetch = resolve;
      }),
    );
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);
    expect(fetchMock()).toHaveBeenCalledTimes(1);

    // signOut lands while the POST is in flight.
    authStore.setUnauthenticated();
    resolveFetch(jsonResponse(REFRESH_OK_1));
    await vi.advanceTimersByTimeAsync(0);

    // The stale result is neither folded back into the store nor fanned out.
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(localStorage.getItem(FALLBACK_MSG_KEY)).toBeNull();
  });

  it("stops (and releases the lease) when the store goes unauthenticated, e.g. signOut", async () => {
    authStore.setAuthenticated(session());
    stop = startSessionRefresh();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(localStorage.getItem(LEASE_KEY)).toContain('"id"');

    authStore.setUnauthenticated();

    expect(localStorage.getItem(LEASE_KEY)).toBeNull();
    await vi.advanceTimersByTimeAsync(3_600_000);
    expect(fetchMock()).not.toHaveBeenCalled();
  });

  it("uses BroadcastChannel for fan-out when available (no localStorage message)", async () => {
    const posted: unknown[] = [];
    class FakeBroadcastChannel {
      onmessage: ((e: MessageEvent) => void) | null = null;
      postMessage(msg: unknown): void {
        posted.push(msg);
      }
      close(): void {
        // no-op
      }
    }
    vi.stubGlobal("BroadcastChannel", FakeBroadcastChannel);

    authStore.setAuthenticated(session());
    fetchMock().mockResolvedValueOnce(jsonResponse(REFRESH_OK_1));
    stop = startSessionRefresh();

    await vi.advanceTimersByTimeAsync(100_000);
    expect(posted).toEqual([{ kind: "refreshed", expiresAt: T0 + 700, refreshAt: T0 + 617 }]);
    expect(localStorage.getItem(FALLBACK_MSG_KEY)).toBeNull();
  });
});
