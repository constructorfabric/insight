import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseMutationResult,
} from "@tanstack/react-query";

import {
  grantPersonRole,
  listPersonRoles,
  revokePersonRole,
  type PersonRole,
} from "@/api/identity-client";

import { ADMIN_ROLE_ID } from "./identity-me";

const PERSON_ROLES_KEY = ["identity", "person-roles"] as const;

const personRolesKey = (personId: string) =>
  [...PERSON_ROLES_KEY, personId] as const;

export interface PersonAdminRole {
  isAdmin: boolean;
  /** The assignment's own id — what a revoke addresses, not the person's. */
  personRoleId: string | null;
  isPending: boolean;
  /** The read failed. Distinct from "not an admin", which no verb may assume. */
  isUnknown: boolean;
}

export function usePersonAdminRole(
  personId: string | null | undefined
): PersonAdminRole {
  const held = useQuery({
    queryKey: personRolesKey(personId ?? ""),
    queryFn: ({ signal }) => listPersonRoles(personId as string, signal),
    enabled: personId != null && personId !== "",
  });

  const admin = held.data?.find(
    (role: PersonRole) => role.role_id === ADMIN_ROLE_ID
  );

  return {
    isAdmin: admin != null,
    personRoleId: admin?.person_role_id ?? null,
    isPending: held.isPending,
    isUnknown: held.isError,
  };
}

export function useGrantAdmin(
  personId: string
): UseMutationResult<PersonRole, unknown, void> {
  const client = useQueryClient();
  return useMutation({
    mutationFn: () =>
      grantPersonRole({ person_id: personId, role_id: ADMIN_ROLE_ID }),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: personRolesKey(personId) });
    },
  });
}

export function useRevokeAdmin(
  personId: string
): UseMutationResult<void, unknown, string> {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (personRoleId: string) => revokePersonRole(personRoleId),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: personRolesKey(personId) });
    },
  });
}
