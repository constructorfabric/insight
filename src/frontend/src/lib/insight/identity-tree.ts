import type { PeopleListItem } from "@/api/identity-client";
import type { IdentityPerson } from "@/types/insight";

const toLower = (s: string | undefined | null) => (s ?? "").toLowerCase();

export function rosterTree(
  roster: readonly PeopleListItem[],
  viewerPersonId: string,
): IdentityPerson | null {
  const byId = new Map(
    roster.map((person) => [toLower(person.person_id), person]),
  );
  const children = new Map<string, PeopleListItem[]>();

  for (const person of roster) {
    const managerId = toLower(person.manager_person_id);
    if (!managerId || !byId.has(managerId)) continue;
    const reports = children.get(managerId) ?? [];
    reports.push(person);
    children.set(managerId, reports);
  }

  const build = (
    person: PeopleListItem,
    ancestors: ReadonlySet<string>,
  ): IdentityPerson => {
    const personId = toLower(person.person_id);
    const nextAncestors = new Set(ancestors).add(personId);
    const manager = person.manager_person_id
      ? byId.get(toLower(person.manager_person_id))
      : undefined;
    const reports = (children.get(personId) ?? []).filter(
      (report) => !nextAncestors.has(toLower(report.person_id)),
    );

    return {
      person_id: person.person_id,
      email: person.email ?? "",
      display_name: person.display_name ?? "",
      first_name: person.first_name ?? undefined,
      last_name: person.last_name ?? undefined,
      username: person.username ?? undefined,
      department: person.attributes.department,
      division: person.attributes.division,
      job_title: person.attributes.job_title,
      status: person.attributes.status,
      parent_person_id: person.manager_person_id,
      supervisor_name:
        manager?.display_name?.trim() || manager?.username?.trim() || null,
      subordinates: reports.map((report) => build(report, nextAncestors)),
    };
  };

  const viewer = byId.get(toLower(viewerPersonId));
  return viewer ? build(viewer, new Set()) : null;
}

export function findIdentityNode(
  tree: IdentityPerson | null | undefined,
  personId: string,
): IdentityPerson | null {
  if (!tree) return null;
  const target = toLower(personId);
  if (toLower(tree.person_id) === target) return tree;
  for (const sub of tree.subordinates) {
    const found = findIdentityNode(sub, target);
    if (found) return found;
  }
  return null;
}

export interface RosterEntry {
  /** Canonical person id — the key for links, metric ids and React keys. */
  person_id: string;
  email: string;
  display_name: string;
  username: string;
  supervisor_person_id: string | null;
  /** True when the person is a direct report of the pivot (depth 1). */
  is_direct: boolean;
}

/**
 * Flatten a pivot's transitive subordinates into a roster.
 *
 * The pivot itself is excluded — Team Lead and exec drill targets read their
 * own metrics on their personal dashboard, not in the team table.
 */
export function flattenSubordinates(pivot: IdentityPerson): RosterEntry[] {
  const out: RosterEntry[] = [];
  const walk = (
    node: IdentityPerson,
    supervisorPersonId: string,
    isDirect: boolean,
  ): void => {
    for (const sub of node.subordinates) {
      out.push({
        person_id: sub.person_id,
        email: sub.email,
        display_name: sub.display_name,
        username: sub.username ?? "",
        supervisor_person_id: supervisorPersonId,
        is_direct: isDirect,
      });
      walk(sub, sub.person_id, false);
    }
  };
  walk(pivot, pivot.person_id, true);
  return out;
}

/**
 * Narrow a roster to the pivot's direct reports when `directOnly` is set.
 *
 * `null` passes through unchanged so callers keep their "roster not loaded
 * yet" gate regardless of the toggle state.
 */
export function scopeRosterToDirectReports(
  roster: RosterEntry[] | null,
  directOnly: boolean,
): RosterEntry[] | null {
  if (!roster || !directOnly) return roster;
  return roster.filter((r) => r.is_direct);
}

/**
 * True when the roster has at least one indirect report. When every entry is
 * direct, scoping to direct reports cannot change the roster, so the
 * "Direct reports only" toggle would be a no-op and should be hidden.
 */
export function hasIndirectReports(roster: RosterEntry[] | null): boolean {
  return roster?.some((r) => !r.is_direct) ?? false;
}
