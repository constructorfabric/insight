import { useQuery } from "@tanstack/react-query";

import { searchPersons } from "@/api/identity-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import { personName } from "@/lib/identities/person-display";
import { normalizePersonId } from "@/lib/metrics/entity";
import { useMe } from "@/queries/identity-me";

export function useConnectorScheduleAuthor(
  personId: string | undefined
): string {
  const { session } = useAuth();
  const scope = sessionAuthorizationScope(session);
  const me = useMe();
  const id = personId === undefined ? undefined : normalizePersonId(personId);
  const self = id !== undefined && id === me.data?.person_id;
  const person = useQuery({
    queryKey: ["identity", "schedule-author", scope, id],
    queryFn: async ({ signal }) => {
      const found = await searchPersons(id!, { limit: 1 }, signal);
      const author = found.items.find(
        (item) => normalizePersonId(item.person_id) === id
      );
      return author ? personName(author) : null;
    },
    enabled:
      scope != null && me.data !== undefined && id !== undefined && !self,
    staleTime: 60_000,
    retry: false,
  });

  if (self) return "you";
  return (!person.isError && person.data) || "another admin";
}
