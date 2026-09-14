// @vitest-environment jsdom
/**
 * A subject's admin role, as the person window reads and writes it. What
 * matters: "holds nothing" and "may not ask" are different answers, so a
 * refused read never renders as "not an admin"; the query stays idle until a
 * person is named; and a grant or revoke invalidates the read that drew the
 * badge, so the window reflects the write without a reload.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as identityClient from "@/api/identity-client";

vi.mock("@/api/identity-client");

import { ADMIN_ROLE_ID } from "./identity-me";
import {
  useGrantAdmin,
  usePersonAdminRole,
  useRevokeAdmin,
} from "./person-roles";

const listPersonRoles = vi.mocked(identityClient.listPersonRoles);
const grantPersonRole = vi.mocked(identityClient.grantPersonRole);
const revokePersonRole = vi.mocked(identityClient.revokePersonRole);

const PERSON = "019e27bc-dec0-7626-81a9-c5524662a6a9";

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper, queryClient };
}

function assignment(roleId: string): identityClient.PersonRole {
  return {
    person_role_id: "019e27bc-0000-7000-8000-000000000001",
    person_id: PERSON,
    role_id: roleId,
    valid_to: null,
  };
}

beforeEach(() => {
  vi.resetAllMocks();
});

describe("usePersonAdminRole", () => {
  it("reports the active admin assignment and its id", async () => {
    listPersonRoles.mockResolvedValueOnce([assignment(ADMIN_ROLE_ID)]);

    const { wrapper } = harness();
    const { result } = renderHook(() => usePersonAdminRole(PERSON), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isAdmin).toBe(true));
    expect(result.current.personRoleId).toBe(
      "019e27bc-0000-7000-8000-000000000001"
    );
    expect(result.current.isUnknown).toBe(false);
  });

  it("reports not-admin when the person holds some other role", async () => {
    listPersonRoles.mockResolvedValueOnce([assignment("some-other-role")]);

    const { wrapper } = harness();
    const { result } = renderHook(() => usePersonAdminRole(PERSON), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.isAdmin).toBe(false);
    expect(result.current.personRoleId).toBeNull();
    expect(result.current.isUnknown).toBe(false);
  });

  it("reports unknown rather than not-admin when the read is refused", async () => {
    listPersonRoles.mockRejectedValueOnce(
      new identityClient.IdentityApiError(403, { error: "forbidden" })
    );

    const { wrapper } = harness();
    const { result } = renderHook(() => usePersonAdminRole(PERSON), {
      wrapper,
    });

    await waitFor(() => expect(result.current.isUnknown).toBe(true));
    expect(result.current.isAdmin).toBe(false);
  });

  it("does not ask until a person is named", () => {
    const { wrapper } = harness();
    renderHook(() => usePersonAdminRole(null), { wrapper });

    expect(listPersonRoles).not.toHaveBeenCalled();
  });
});

describe("useGrantAdmin", () => {
  it("grants the seeded admin role to the named person", async () => {
    grantPersonRole.mockResolvedValueOnce(assignment(ADMIN_ROLE_ID));

    const { wrapper } = harness();
    const { result } = renderHook(() => useGrantAdmin(PERSON), { wrapper });
    result.current.mutate();

    await waitFor(() => expect(grantPersonRole).toHaveBeenCalled());
    expect(grantPersonRole).toHaveBeenCalledWith({
      person_id: PERSON,
      role_id: ADMIN_ROLE_ID,
    });
  });

  it("re-reads the subject's roles once the grant lands", async () => {
    grantPersonRole.mockResolvedValueOnce(assignment(ADMIN_ROLE_ID));
    listPersonRoles.mockResolvedValue([assignment(ADMIN_ROLE_ID)]);

    const { wrapper } = harness();
    const { result } = renderHook(
      () => ({
        held: usePersonAdminRole(PERSON),
        grant: useGrantAdmin(PERSON),
      }),
      { wrapper }
    );

    await waitFor(() => expect(listPersonRoles).toHaveBeenCalledTimes(1));
    result.current.grant.mutate();

    await waitFor(() => expect(listPersonRoles).toHaveBeenCalledTimes(2));
  });
});

describe("useRevokeAdmin", () => {
  it("revokes the assignment by its own id", async () => {
    revokePersonRole.mockResolvedValueOnce(undefined);

    const { wrapper } = harness();
    const { result } = renderHook(() => useRevokeAdmin(PERSON), { wrapper });
    result.current.mutate("019e27bc-0000-7000-8000-000000000001");

    await waitFor(() => expect(revokePersonRole).toHaveBeenCalled());
    expect(revokePersonRole).toHaveBeenCalledWith(
      "019e27bc-0000-7000-8000-000000000001"
    );
  });
});
