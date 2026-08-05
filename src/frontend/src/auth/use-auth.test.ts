import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { authStore } from "./auth-store";
import { signOut } from "./use-auth";
import { makeSession } from "@/test/session";

const fetchMock = () => globalThis.fetch as ReturnType<typeof vi.fn>;

describe("signIn", () => {
  let assign: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.resetModules();
    assign = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, assign },
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  async function freshSignIn() {
    const mod = await import("./use-auth");
    return mod.signIn;
  }

  it("encodes the current path into return_to", async () => {
    const signIn = await freshSignIn();
    signIn("/ic/bob?period=month");
    expect(assign).toHaveBeenCalledWith(
      `/auth/login?return_to=${encodeURIComponent("/ic/bob?period=month")}`
    );
  });

  it("collapses /auth paths to / so the login redirect cannot nest", async () => {
    const signIn = await freshSignIn();
    signIn("/auth/login?return_to=%2Fauth%2Flogin");
    expect(assign).toHaveBeenCalledWith("/auth/login?return_to=%2F");
  });

  it("rejects non-relative and protocol-relative destinations", async () => {
    const signIn = await freshSignIn();
    signIn("//evil.example/phish");
    expect(assign).toHaveBeenCalledWith("/auth/login?return_to=%2F");
  });

  it("defaults return_to to the current path, query, and hash", async () => {
    Object.defineProperty(window, "location", {
      configurable: true,
      value: {
        ...window.location,
        pathname: "/board",
        search: "?tab=1",
        hash: "#row-9",
        assign,
      },
    });
    const signIn = await freshSignIn();
    signIn();
    expect(assign).toHaveBeenCalledWith(
      `/auth/login?return_to=${encodeURIComponent("/board?tab=1#row-9")}`
    );
  });

  it("does not stack redirects while one is in flight", async () => {
    const signIn = await freshSignIn();
    signIn("/a");
    signIn("/b");
    expect(assign).toHaveBeenCalledTimes(1);
  });
});

describe("signOut", () => {
  let assign: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    authStore.reset();
    authStore.setAuthenticated(makeSession({ email: "bob@example.com" }));
    vi.stubGlobal("fetch", vi.fn());
    assign = vi.fn();
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { ...window.location, assign },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    authStore.reset();
  });

  it("POSTs /auth/logout, clears the session, and follows rp_logout_url", async () => {
    fetchMock().mockResolvedValueOnce({
      json: async () => ({ rp_logout_url: "https://idp.example/logout" }),
    });

    await signOut();

    const [url, init] = fetchMock().mock.calls[0];
    expect(url).toBe("/auth/logout");
    expect(init).toMatchObject({ method: "POST", credentials: "include" });
    expect(init.headers).toMatchObject({ "X-CSRF-Token": "csrf-1" });
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(assign).toHaveBeenCalledWith("https://idp.example/logout");
  });

  it("falls back to / when no rp_logout_url is returned", async () => {
    fetchMock().mockResolvedValueOnce({ json: async () => ({}) });

    await signOut();

    expect(assign).toHaveBeenCalledWith("/");
    expect(authStore.getSnapshot().status).toBe("unauthenticated");
  });

  it("still clears the session and bounces to / on a network error", async () => {
    fetchMock().mockRejectedValueOnce(new Error("network down"));

    await signOut();

    expect(authStore.getSnapshot().status).toBe("unauthenticated");
    expect(assign).toHaveBeenCalledWith("/");
  });
});
