import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { authStore } from "./auth-store";
import { loadSession } from "./session";

const fetchMock = () => globalThis.fetch as ReturnType<typeof vi.fn>;

describe("loadSession", () => {
  beforeEach(() => {
    authStore.reset();
    vi.stubGlobal("fetch", vi.fn());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    authStore.reset();
  });

  it("populates the store from a 200 /auth/me and returns authenticated", async () => {
    fetchMock().mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        user: "p-1",
        email: "bob@example.com",
        tenant_id: "t-1",
        roles: ["user"],
        csrf_token: "csrf-1",
        expires_at: 1770000600,
        refresh_at: 1770000510,
      }),
    });

    const status = await loadSession();

    expect(status).toBe("authenticated");
    const snap = authStore.getSnapshot();
    expect(snap.status).toBe("authenticated");
    expect(snap.session).toEqual({
      personId: "p-1",
      email: "bob@example.com",
      tenantId: "t-1",
      roles: ["user"],
      csrfToken: "csrf-1",
      expiresAt: 1770000600,
      refreshAt: 1770000510,
      impersonatorEmail: null,
    });
    const [url, init] = fetchMock().mock.calls[0];
    expect(url).toBe("/auth/me");
    expect(init).toMatchObject({ credentials: "include" });
  });

  it("defaults missing fields to empty values (csrf_token present)", async () => {
    fetchMock().mockResolvedValueOnce({
      ok: true,
      json: async () => ({ csrf_token: "csrf-1" }),
    });

    await loadSession();

    // expiresAt/refreshAt default to 0 — the refresh driver treats that as
    // "never schedule" (pre-timestamp backend), not as "refresh now".
    expect(authStore.getSnapshot().session).toEqual({
      personId: "",
      email: "",
      tenantId: "",
      roles: [],
      csrfToken: "csrf-1",
      expiresAt: 0,
      refreshAt: 0,
      impersonatorEmail: null,
    });
  });

  it("surfaces impersonator_email from a view-as session (insight#1941)", async () => {
    fetchMock().mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        user: "p-target",
        email: "target@example.com",
        csrf_token: "csrf-1",
        impersonator_email: "admin@example.com",
      }),
    });

    await loadSession();

    expect(authStore.getSnapshot().session).toMatchObject({
      email: "target@example.com",
      impersonatorEmail: "admin@example.com",
    });
  });

  it("stores malformed timing fields as 0 (driver never schedules from them)", async () => {
    fetchMock().mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        csrf_token: "csrf-1",
        expires_at: "1770000600", // wire contract violation: string, not number
        refresh_at: Number.NaN,
      }),
    });

    await loadSession();

    expect(authStore.getSnapshot().session).toMatchObject({
      expiresAt: 0,
      refreshAt: 0,
    });
  });

  it("fails closed to unauthenticated when /auth/me omits csrf_token", async () => {
    // A live session always carries a CSRF token; without it, state-changing
    // /auth/* would be rejected server-side — so treat it as no session.
    fetchMock().mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        user: "p-1",
        email: "bob@example.com",
        tenant_id: "t-1",
        roles: ["user"],
      }),
    });

    const status = await loadSession();

    expect(status).toBe("unauthenticated");
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
  });

  it("fails closed to unauthenticated on a non-ok response", async () => {
    fetchMock().mockResolvedValueOnce({ ok: false, json: async () => ({}) });

    const status = await loadSession();

    expect(status).toBe("unauthenticated");
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
  });

  it("fails closed on a network error reaching the authenticator", async () => {
    fetchMock().mockRejectedValueOnce(new Error("network down"));

    const status = await loadSession();

    expect(status).toBe("unauthenticated");
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
  });
});
