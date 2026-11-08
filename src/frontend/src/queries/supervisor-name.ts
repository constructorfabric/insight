import { useQuery } from "@tanstack/react-query";

import { getSupervisor } from "@/api/identity-client";
import { personName } from "@/lib/identities/person-display";

const SUPERVISOR_STALE_TIME = 60 * 1000;

export function useSupervisorName(personId: string | null): string | null {
  const query = useQuery({
    queryKey: ["identity", "supervisor", personId],
    queryFn: ({ signal }) => getSupervisor(personId ?? "", signal),
    staleTime: SUPERVISOR_STALE_TIME,
    enabled: personId != null,
  });

  return query.data ? personName(query.data) : null;
}
