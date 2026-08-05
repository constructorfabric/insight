/**
 * Mock Data Registry — Single Source of Truth
 *
 * All mock data derives from these definitions. Every handler, factory,
 * and identity service must reference this registry instead of having
 * its own hardcoded lists.
 *
 * Org structure follows the real backend shape: supervisor_person_id forms a
 * recursive identity tree, no slug-based "team" grouping. A "team" is the
 * set of direct (or transitive) reports of a given supervisor. Person ids are
 * UUIDs (the identity-cutover key), derived from the email so fixtures stay
 * readable; emails ride along for display and legacy-URL tests.
 */

export type MockPerson = {
  /** Canonical person id (UUID) — the key routes, metric ids and lookups use. */
  person_id: string;
  /** Display + legacy-URL identity; no longer a key. */
  email: string;
  name: string;
  department: string;
  role: string;
  seniority: string;
  is_lead: boolean;
  ai_tools: string[];
  supervisor_person_id: string | null;
};

export const PEOPLE: MockPerson[] = [
  { person_id: 'e8a33e91-2658-58dc-8175-ebf473d8be5c', email: 'carol.chen@example.com',    name: 'Carol Chen',    department: 'Engineering', role: 'Tech Lead',                seniority: 'Staff',     is_lead: true,  ai_tools: ['Codex'],                 supervisor_person_id: null },
  { person_id: '2b6c1926-f785-5693-ac00-bdee2729044d', email: 'bob.park@example.com',      name: 'Bob Park',      department: 'Engineering', role: 'Tech Lead',                seniority: 'Staff',     is_lead: true,  ai_tools: ['Cursor'],                supervisor_person_id: 'e8a33e91-2658-58dc-8175-ebf473d8be5c' },
  { person_id: '2517cd48-4961-52b3-a401-b0e5a03858a4', email: 'alice.kim@example.com',     name: 'Alice Kim',     department: 'Engineering', role: 'Senior Software Engineer', seniority: 'Senior',    is_lead: false, ai_tools: ['Cursor', 'Claude Code'], supervisor_person_id: '2b6c1926-f785-5693-ac00-bdee2729044d' },
  { person_id: 'f3b862b0-e9d5-500a-a9f5-b3219e26a80c', email: 'david.liu@example.com',     name: 'David Liu',     department: 'Engineering', role: 'Junior Software Engineer', seniority: 'Junior',    is_lead: false, ai_tools: ['Cursor'],                supervisor_person_id: '2b6c1926-f785-5693-ac00-bdee2729044d' },
  { person_id: '2ec567e8-285c-5ca7-b6bf-ea99218b1116', email: 'eve.novak@example.com',     name: 'Eve Novak',     department: 'Engineering', role: 'Staff Software Engineer',  seniority: 'Staff',     is_lead: false, ai_tools: ['Cursor', 'Claude Code'], supervisor_person_id: '2b6c1926-f785-5693-ac00-bdee2729044d' },
  { person_id: '6bba81f8-d61e-5a14-8799-e7eb0798a339', email: 'grace.wu@example.com',      name: 'Grace Wu',      department: 'Engineering', role: 'Software Engineer',        seniority: 'Mid',       is_lead: false, ai_tools: ['Cursor'],                supervisor_person_id: '2b6c1926-f785-5693-ac00-bdee2729044d' },
  { person_id: '704c60df-0ddf-5987-81ad-883413e3f420', email: 'iris.tan@example.com',      name: 'Iris Tan',      department: 'Engineering', role: 'Junior Software Engineer', seniority: 'Junior',    is_lead: false, ai_tools: [],                        supervisor_person_id: '2b6c1926-f785-5693-ac00-bdee2729044d' },
  { person_id: 'a57327fb-87da-5d93-ae0e-e97e16c27756', email: 'leo.dunn@example.com',      name: 'Leo Dunn',      department: 'Engineering', role: 'Junior Software Engineer', seniority: 'Junior',    is_lead: false, ai_tools: ['Cursor'],                supervisor_person_id: '2b6c1926-f785-5693-ac00-bdee2729044d' },
  { person_id: '2eed8dd1-5af3-5785-a432-e4a6703c59ec', email: 'frank.moss@example.com',    name: 'Frank Moss',    department: 'Engineering', role: 'Tech Lead',                seniority: 'Principal', is_lead: true,  ai_tools: ['Claude Code'],           supervisor_person_id: 'e8a33e91-2658-58dc-8175-ebf473d8be5c' },
  { person_id: '113fccc7-7f5e-5827-9071-8bc968050444', email: 'hank.reed@example.com',     name: 'Hank Reed',     department: 'Engineering', role: 'Tech Lead',                seniority: 'Senior',    is_lead: true,  ai_tools: ['Cursor', 'Codex'],       supervisor_person_id: 'e8a33e91-2658-58dc-8175-ebf473d8be5c' },
  { person_id: '8e2121a4-c3bf-56a8-a334-372694a46702', email: 'jake.fox@example.com',      name: 'Jake Fox',      department: 'Engineering', role: 'Tech Lead',                seniority: 'Mid',       is_lead: true,  ai_tools: ['Cursor'],                supervisor_person_id: 'e8a33e91-2658-58dc-8175-ebf473d8be5c' },
  { person_id: 'a9558c56-1865-5fb9-ba46-c0fee39380c0', email: 'kira.sato@example.com',     name: 'Kira Sato',     department: 'Engineering', role: 'Tech Lead',                seniority: 'Senior',    is_lead: true,  ai_tools: ['Claude Code', 'Codex'],  supervisor_person_id: 'e8a33e91-2658-58dc-8175-ebf473d8be5c' },
  { person_id: '1c2895dc-8646-5a09-877f-580313f59aab', email: 'noah.bell@example.com',     name: 'Noah Bell',     department: 'Engineering', role: 'Senior Software Engineer', seniority: 'Senior',    is_lead: false, ai_tools: ['Cursor', 'Claude Code'], supervisor_person_id: '2517cd48-4961-52b3-a401-b0e5a03858a4' },
  { person_id: '54bbe402-71ef-5d98-92c0-4a59415799a1', email: 'olivia.park@example.com',   name: 'Olivia Park',   department: 'Engineering', role: 'Software Engineer',        seniority: 'Mid',       is_lead: false, ai_tools: ['Claude Code'],           supervisor_person_id: '2517cd48-4961-52b3-a401-b0e5a03858a4' },
  { person_id: '7530bc37-fa46-587e-8f54-1096225d35c4', email: 'priya.shah@example.com',    name: 'Priya Shah',    department: 'Engineering', role: 'Software Engineer',        seniority: 'Mid',       is_lead: false, ai_tools: ['Cursor'],                supervisor_person_id: '2eed8dd1-5af3-5785-a432-e4a6703c59ec' },
  { person_id: '54026038-cd9c-5a1f-8c0c-8cbeeb2cd6df', email: 'quinn.lee@example.com',     name: 'Quinn Lee',     department: 'Engineering', role: 'Junior Software Engineer', seniority: 'Junior',    is_lead: false, ai_tools: [],                        supervisor_person_id: '2eed8dd1-5af3-5785-a432-e4a6703c59ec' },
  { person_id: 'bf45a898-0c26-5afd-9b45-1f41167bc831', email: 'ravi.iyer@example.com',     name: 'Ravi Iyer',     department: 'Engineering', role: 'Senior Software Engineer', seniority: 'Senior',    is_lead: false, ai_tools: ['Codex'],                 supervisor_person_id: '113fccc7-7f5e-5827-9071-8bc968050444' },
  { person_id: 'cbc1137b-0f5a-53b7-a3fc-eae017dda5b4', email: 'sara.bishop@example.com',   name: 'Sara Bishop',   department: 'Engineering', role: 'Software Engineer',        seniority: 'Mid',       is_lead: false, ai_tools: ['Cursor', 'Codex'],       supervisor_person_id: '113fccc7-7f5e-5827-9071-8bc968050444' },
  { person_id: 'e99a7f65-76a8-5548-9ad4-f56a8a3fe59b', email: 'tom.alvarez@example.com',   name: 'Tom Alvarez',   department: 'Engineering', role: 'Software Engineer',        seniority: 'Mid',       is_lead: false, ai_tools: ['Cursor'],                supervisor_person_id: '8e2121a4-c3bf-56a8-a334-372694a46702' },
  { person_id: 'f37e6f3c-b881-5a2d-a7a9-c3a54f7de79f', email: 'dave.sales@example.com',    name: 'Dave Hart',     department: 'Sales',       role: 'Account Executive',        seniority: 'Senior',    is_lead: false, ai_tools: [],                        supervisor_person_id: null },
];

const PEOPLE_BY_ID: Record<string, MockPerson> = Object.fromEntries(
  PEOPLE.map((p) => [p.person_id, p]),
);

function directReports(supervisorPersonId: string): MockPerson[] {
  return PEOPLE.filter((p) => p.supervisor_person_id === supervisorPersonId);
}

// Keyed by the lowercased email: identity compares case-insensitively, so an
// exact-case mock would refuse spellings the real service resolves.
export const PEOPLE_BY_EMAIL: Record<string, MockPerson> = Object.fromEntries(
  PEOPLE.map((p) => [p.email.toLowerCase(), p]),
);

export type MockIdentityRaw = {
  person_id: string;
  email: string;
  display_name: string;
  first_name: string;
  last_name: string;
  department: string;
  division: string;
  job_title: string;
  status: string;
  supervisor_email: string | null;
  supervisor_name: string | null;
  subordinates: MockIdentityRaw[];
};

export function buildIdentityTree(personId: string): MockIdentityRaw | null {
  const root = PEOPLE_BY_ID[personId];
  if (!root) return null;

  const splitName = (name: string): { first: string; last: string } => {
    const idx = name.indexOf(' ');
    return idx < 0
      ? { first: name, last: '' }
      : { first: name.slice(0, idx), last: name.slice(idx + 1) };
  };

  const toRaw = (p: MockPerson): MockIdentityRaw => {
    const { first, last } = splitName(p.name);
    const sup = p.supervisor_person_id
      ? PEOPLE_BY_ID[p.supervisor_person_id]
      : null;
    return {
      person_id: p.person_id,
      email: p.email,
      display_name: p.name,
      first_name: first,
      last_name: last,
      department: p.department,
      division: p.department,
      job_title: p.role,
      status: 'Active',
      supervisor_email: sup?.email ?? null,
      supervisor_name: sup?.name ?? null,
      subordinates: directReports(p.person_id).map(toRaw),
    };
  };

  return toRaw(root);
}
