/**
 * The admin gate. What matters: it fails CLOSED — no session, a pending
 * fetch, an error, or an empty grant list must all render as "not an admin";
 * only the seeded role id opens it (the realm role name never appears here);
 * and a session change keys a fresh cache entry so a sign-out/sign-in never
 * inherits the previous caller's answer.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as identityClient from "@/api/identity-client";

vi.mock("@/api/identity-client");

const session = vi.hoisted(() => ({ value: { scope: "tenant-a" } as unknown }));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: session.value }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (s: unknown) =>
    s == null ? null : (s as { scope: string }).scope,
}));

import { ADMIN_ROLE_ID, useIsAdmin } from "./identity-me";

const getMe = vi.mocked(identityClient.getMe);

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper };
}

function me(roles: identityClient.MeRole[]): identityClient.MeResponse {
  return { person_id: "p-1", insight_tenant_id: "t-1", roles };
}

beforeEach(() => {
  vi.resetAllMocks();
  session.value = { scope: "tenant-a" };
});

describe("useIsAdmin", () => {
  it("opens only for the seeded admin role id", async () => {
    getMe.mockResolvedValueOnce(
      me([{ role_id: ADMIN_ROLE_ID, name: "renamed-later" }]),
    );

    const { result } = renderHook(() => useIsAdmin(), harness());

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.isAdmin).toBe(true);
  });

  it("stays closed for other roles, whatever they are named", async () => {
    getMe.mockResolvedValueOnce(
      me([{ role_id: "01900000-0000-7000-8000-00000000aaaa", name: "admin" }]),
    );

    const { result } = renderHook(() => useIsAdmin(), harness());

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.isAdmin).toBe(false);
  });

  it("reads an empty grant list as not-an-admin, not as an error", async () => {
    getMe.mockResolvedValueOnce(me([]));

    const { result } = renderHook(() => useIsAdmin(), harness());

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.isAdmin).toBe(false);
  });

  it("fails closed while pending and on a fetch error", async () => {
    getMe.mockRejectedValueOnce(new Error("identity is down"));

    const { result } = renderHook(() => useIsAdmin(), harness());

    expect(result.current.isAdmin).toBe(false);
    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.isAdmin).toBe(false);
  });

  it("names a failed check as an error, distinct from a confirmed non-admin", async () => {
    getMe.mockRejectedValueOnce(new Error("identity is down"));
    getMe.mockResolvedValueOnce(me([{ role_id: ADMIN_ROLE_ID, name: "admin" }]));

    const { result } = renderHook(() => useIsAdmin(), harness());

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.isAdmin).toBe(false);

    // retry() re-asks and the gate opens once the answer lands — an errored
    // check must never be terminal for the session.
    result.current.retry();
    await waitFor(() => expect(result.current.isAdmin).toBe(true));
    expect(result.current.isError).toBe(false);
  });

  it("reports no error for a confirmed non-admin", async () => {
    getMe.mockResolvedValueOnce(me([]));

    const { result } = renderHook(() => useIsAdmin(), harness());

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.isError).toBe(false);
  });

  it("never asks without a session, and stays closed", async () => {
    session.value = null;

    const { result } = renderHook(() => useIsAdmin(), harness());

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(getMe).not.toHaveBeenCalled();
    expect(result.current.isAdmin).toBe(false);
  });
});
