import { useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";

import { getPerson } from "@/api/identity-client";
import { getViewerPersonId } from "@/auth";
import { findIdentityNode } from "@/lib/insight/identity-tree";
import { normalizePersonId } from "@/lib/metrics/entity";
import type { IdentityPerson } from "@/types/insight";

export function useIcPerson(personId: string): UseQueryResult<IdentityPerson> {
  const queryClient = useQueryClient();
  const key = normalizePersonId(personId);
  return useQuery({
    queryKey: ["identity", "person", key],
    queryFn: () => getPerson(personId),
    enabled: Boolean(personId),
    // A subordinate's identity (name, title, own subtree) already rides the
    // viewer's cached org tree — surface it immediately instead of waiting
    // for the canonical per-person fetch, which still runs and replaces it.
    placeholderData: () => {
      const viewerKey = normalizePersonId(getViewerPersonId() ?? "");
      if (!viewerKey || viewerKey === key) return undefined;
      const viewerTree = queryClient.getQueryData<IdentityPerson>([
        "identity",
        "person",
        viewerKey,
      ]);
      return viewerTree
        ? (findIdentityNode(viewerTree, personId) ?? undefined)
        : undefined;
    },
  });
}
