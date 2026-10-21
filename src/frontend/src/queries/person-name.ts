import { queryOptions, useQuery } from "@tanstack/react-query";

import { searchPersons } from "@/api/identity-client";
import { personName } from "@/lib/identities/person-display";
import { normalizePersonId } from "@/lib/metrics/entity";

/**
 * The name one person goes by, read through the administrator person search,
 * which names anyone in the tenant by id. Null when the journal knows no name.
 */
export function personNameQuery(personId: string) {
  const id = normalizePersonId(personId);

  return queryOptions({
    queryKey: ["identity", "person-name", id],
    queryFn: async ({ signal }) => {
      const found = await searchPersons(id, { limit: 1 }, signal);
      const person = found.items.find(
        (item) => normalizePersonId(item.person_id) === id
      );

      return person ? personName(person) : null;
    },
  });
}

export function usePersonName(personId: string | null) {
  return useQuery({
    ...personNameQuery(personId ?? ""),
    enabled: personId != null && personId !== "",
  });
}
