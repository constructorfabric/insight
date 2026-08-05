import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  createSavedQuery,
  deleteSavedQuery,
  getSavedQuery,
  listSavedQueries,
  runSavedQuery,
  updateSavedQuery,
  type CreateSavedQueryRequest,
  type RunResponse,
  type RunSavedQueryRequest,
  type SavedQuery,
  type SavedQuerySummary,
  type UpdateSavedQueryRequest,
} from "@/api/saved-queries-client";

const LIST_KEY = ["saved-queries"] as const;

export function useSavedQueries(): UseQueryResult<SavedQuerySummary[]> {
  return useQuery({
    queryKey: LIST_KEY,
    queryFn: listSavedQueries,
    select: (data) => data.items,
  });
}

export function useSavedQuery(
  id: string | null
): UseQueryResult<SavedQuery> {
  return useQuery({
    queryKey: ["saved-queries", id],
    queryFn: () => getSavedQuery(id as string),
    enabled: id !== null,
  });
}

export function useCreateSavedQuery() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateSavedQueryRequest) => createSavedQuery(body),
    onSuccess: () => client.invalidateQueries({ queryKey: LIST_KEY }),
  });
}

export function useUpdateSavedQuery(id: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: UpdateSavedQueryRequest) => updateSavedQuery(id, body),
    onSuccess: (updated) => {
      client.invalidateQueries({ queryKey: LIST_KEY });
      client.setQueryData(["saved-queries", id], updated);
    },
  });
}

export function useDeleteSavedQuery() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteSavedQuery(id),
    onSuccess: () => client.invalidateQueries({ queryKey: LIST_KEY }),
  });
}

export function useRunSavedQuery(id: string) {
  return useMutation<RunResponse, Error, RunSavedQueryRequest>({
    mutationFn: (body: RunSavedQueryRequest) => runSavedQuery(id, body),
  });
}
