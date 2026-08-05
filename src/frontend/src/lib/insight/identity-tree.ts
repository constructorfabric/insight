import type { IdentityPerson } from "@/types/insight";

const toLower = (s: string | undefined | null) => (s ?? "").toLowerCase();

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
