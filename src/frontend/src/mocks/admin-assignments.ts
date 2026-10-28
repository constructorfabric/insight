import { PEOPLE } from "./registry";

const ADMIN_ROLE_ID = "a4d11000-0000-4000-8000-000000000001";
const TENANT_ID = "00000000-0000-4000-8000-00000000c0de";
const SEEDED_AT = "2026-09-01T09:00:00Z";

export interface MockPersonRole {
  person_role_id: string;
  insight_tenant_id: string;
  person_id: string;
  role_id: string;
  valid_from: string;
  valid_to: string | null;
  author_person_id: string;
  reason: string | null;
  created_at: string;
}

function seeded(personId: string, index: number): [string, MockPersonRole] {
  return [
    personId.toLowerCase(),
    {
      person_role_id: `019e27bc-0000-7000-8000-00000000000${index + 1}`,
      insight_tenant_id: TENANT_ID,
      person_id: personId,
      role_id: ADMIN_ROLE_ID,
      valid_from: SEEDED_AT,
      valid_to: null,
      author_person_id: PEOPLE[0]?.person_id ?? personId,
      reason: null,
      created_at: SEEDED_AT,
    },
  ];
}

/**
 * Two administrators, so the tenant is never admin-less and a dashboard has
 * someone to be shared with; everyone else holds no assignment.
 */
export const ADMIN_ASSIGNMENTS = new Map<string, MockPersonRole>(
  PEOPLE.slice(0, 2).map((person, index) => seeded(person.person_id, index))
);

export function isAdministrator(personId: string): boolean {
  return ADMIN_ASSIGNMENTS.has(personId.toLowerCase());
}
