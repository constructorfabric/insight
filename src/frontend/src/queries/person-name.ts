import { queryOptions, useQuery } from "@tanstack/react-query";

import { getPerson, searchPersons } from "@/api/identity-client";
import { personName } from "@/lib/identities/person-display";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useIsAdmin } from "@/queries/identity-me";

/**
 * Where a name is read: the administrator person search names anyone in the
 * tenant; anyone else reads the roster, which names only who they may see.
 */
export type PersonNameSource = "tenant" | "roster";

/**
 * What to show for one person. The tenant search falls back to the id; the
 * roster answers null for a person it names nothing, so a member never sees
 * a raw id.
 */
export function personNameQuery(
  personId: string,
  source: PersonNameSource = "tenant"
) {
  const id = normalizePersonId(personId);

  return queryOptions({
    queryKey: ["identity", "person-name", source, id],
    queryFn: async ({ signal }) => {
      if (source === "roster") {
        return personName(await getPerson(id, signal));
      }

      const found = await searchPersons(id, { limit: 1 }, signal);
      const person = found.items.find(
        (item) => normalizePersonId(item.person_id) === id
      );

      return (person ? personName(person) : null) ?? id;
    },
  });
}

export function usePersonName(personId: string | null) {
  const { isAdmin, isPending } = useIsAdmin();

  return useQuery({
    ...personNameQuery(personId ?? "", isAdmin ? "tenant" : "roster"),
    enabled: personId != null && personId !== "" && !isPending,
    ...(isAdmin ? {} : { retry: false }),
  });
}
