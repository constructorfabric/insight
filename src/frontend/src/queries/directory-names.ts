import { useQuery } from "@tanstack/react-query";

import { listPeople } from "@/api/identity-client";
import { useAuth } from "@/auth/use-auth";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { personName } from "@/lib/identities/person-display";

const DIRECTORY_STALE_TIME = 60 * 1000;

const NO_NAMES: ReadonlyMap<string, string> = new Map();

async function directoryNames(
  personIds: readonly string[],
  signal: AbortSignal,
): Promise<ReadonlyMap<string, string>> {
  const listed = await listPeople(
    { visibility: "tenant", ids: personIds, limit: personIds.length },
    signal,
  );

  const names = new Map<string, string>();
  for (const person of listed.items) {
    const name = personName(person);
    if (name) names.set(person.person_id.toLowerCase(), name);
  }
  return names;
}

export function useDirectoryNames(
  personIds: readonly string[],
): ReadonlyMap<string, string> {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const query = useQuery({
    queryKey: ["identity", "directory-names", sessionScope, personIds],
    queryFn: ({ signal }) => directoryNames(personIds, signal),
    staleTime: DIRECTORY_STALE_TIME,
    enabled: personIds.length > 0 && sessionScope != null,
  });

  return query.data ?? NO_NAMES;
}
