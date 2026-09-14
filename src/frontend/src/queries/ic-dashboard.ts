import { useQuery, type UseQueryResult } from "@tanstack/react-query";

import { getPerson } from "@/api/identity-client";
import { normalizePersonId } from "@/lib/metrics/entity";
import type { IdentityPerson } from "@/types/insight";

export function useIcPerson(personId: string): UseQueryResult<IdentityPerson> {
  const key = normalizePersonId(personId);
  return useQuery({
    queryKey: ["identity", "person", key],
    queryFn: ({ signal }) => getPerson(personId, signal),
    enabled: Boolean(personId),
  });
}
