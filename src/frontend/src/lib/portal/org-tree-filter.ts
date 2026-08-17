import type { IdentityPerson } from "@/types/insight";

export interface OrgTreeFilter {
  /** Ids to render: every match, and the managers above it. */
  visible: ReadonlySet<string>;
  /** Ids that matched the query themselves. */
  matched: ReadonlySet<string>;
}

function haystack(person: IdentityPerson): string {
  return [
    person.display_name,
    person.job_title,
    person.department,
    person.division,
    person.supervisor_name,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

/**
 * Narrow the org tree to the people a query names and the chain of managers
 * that reaches them.
 *
 * A matched manager does not drag their reports along: a query answered with a
 * whole department buries the person who was asked for. Returns null for a
 * blank query, which leaves the tree unfiltered.
 */
export function filterOrgTree(
  root: IdentityPerson | null,
  query: string
): OrgTreeFilter | null {
  const needle = query.trim().toLowerCase();
  if (!root || needle === "") return null;

  const visible = new Set<string>();
  const matched = new Set<string>();

  const walk = (person: IdentityPerson, ancestors: string[]): void => {
    if (haystack(person).includes(needle)) {
      matched.add(person.person_id);
      visible.add(person.person_id);
      for (const ancestor of ancestors) visible.add(ancestor);
    }
    const deeper = [...ancestors, person.person_id];
    for (const report of person.subordinates) walk(report, deeper);
  };
  walk(root, []);

  return { visible, matched };
}
