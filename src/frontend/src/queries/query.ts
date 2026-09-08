import { useMutation, useQuery, type UseQueryResult } from "@tanstack/react-query";

import {
  listDatasets,
  runQuery,
  type Query,
  type QueryAnswer,
  type QueryDataset,
} from "@/api/query-client";

const DATASETS_KEY = ["query-datasets"] as const;

/** The datasets this build declares; installation-wide, so no session in the key. */
export function useDatasets(): UseQueryResult<QueryDataset[]> {
  return useQuery({
    queryKey: DATASETS_KEY,
    queryFn: listDatasets,
    select: (data) => data.datasets,
    staleTime: Infinity,
  });
}

/** A mutation rather than a query: the answer belongs to the request that ran, not to a cache key. */
export function useRunQuery() {
  return useMutation<QueryAnswer, unknown, Query>({
    mutationFn: runQuery,
  });
}
