import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  createAiContext,
  deleteAiContext,
  deleteAiCredential,
  explainMetric,
  getAiConfig,
  getAiCredentialStatus,
  getAiSettings,
  listAiContext,
  putAiCredential,
  putAiSettings,
  resetAiSettings,
  updateAiContext,
  type AiConfig,
  type AiCredentialStatus,
  type AiSettings,
  type ContextEntry,
  type CreateContextRequest,
  type MetricSnapshot,
  type UpdateContextRequest,
} from "@/api/ai-client";

const CONFIG_KEY = ["ai", "config"] as const;
const CREDENTIAL_KEY = ["ai", "credentials"] as const;
const SETTINGS_KEY = ["ai", "settings"] as const;
const CONTEXT_KEY = ["ai", "context"] as const;

export function useAiConfig(): UseQueryResult<AiConfig> {
  return useQuery({
    queryKey: CONFIG_KEY,
    queryFn: getAiConfig,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

export function useAiCredentialStatus(
  enabled: boolean
): UseQueryResult<AiCredentialStatus> {
  return useQuery({
    queryKey: CREDENTIAL_KEY,
    queryFn: getAiCredentialStatus,
    enabled,
  });
}

/**
 * The two gates the tile affordance waits on: the deployment offers the
 * feature, and this person has stored a key. Both must be true before anything
 * about the assistant is drawn.
 */
export function useAiAvailable(): { featureOn: boolean; hasKey: boolean } {
  const config = useAiConfig();
  const featureOn = config.data?.enabled === true;
  const credential = useAiCredentialStatus(featureOn);

  return { featureOn, hasKey: credential.data?.configured === true };
}

export function useAiSettings(enabled: boolean): UseQueryResult<AiSettings> {
  return useQuery({
    queryKey: SETTINGS_KEY,
    queryFn: getAiSettings,
    enabled,
  });
}

export function useAiContext(
  enabled: boolean
): UseQueryResult<ContextEntry[]> {
  return useQuery({
    queryKey: CONTEXT_KEY,
    queryFn: listAiContext,
    select: (data) => data.items,
    enabled,
  });
}

export function useSaveAiCredential() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (token: string) => putAiCredential(token),
    onSuccess: (status) => {
      client.setQueryData(CREDENTIAL_KEY, status);
    },
  });
}

export function useForgetAiCredential() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: deleteAiCredential,
    onSuccess: () => {
      client.setQueryData(CREDENTIAL_KEY, { configured: false, hint: "" });
    },
  });
}

export function useSaveAiSystemPrompt() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (systemPrompt: string) => putAiSettings(systemPrompt),
    onSuccess: (settings) => client.setQueryData(SETTINGS_KEY, settings),
  });
}

export function useResetAiSystemPrompt() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: resetAiSettings,
    onSuccess: () => client.invalidateQueries({ queryKey: SETTINGS_KEY }),
  });
}

export function useCreateAiContext() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateContextRequest) => createAiContext(body),
    onSuccess: () => client.invalidateQueries({ queryKey: CONTEXT_KEY }),
  });
}

export function useUpdateAiContext() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, body }: { id: string; body: UpdateContextRequest }) =>
      updateAiContext(id, body),
    onSuccess: () => client.invalidateQueries({ queryKey: CONTEXT_KEY }),
  });
}

export function useDeleteAiContext() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => deleteAiContext(id),
    onSuccess: () => client.invalidateQueries({ queryKey: CONTEXT_KEY }),
  });
}

export function useExplainMetric() {
  return useMutation({
    mutationFn: (snapshot: MetricSnapshot) => explainMetric(snapshot),
  });
}
