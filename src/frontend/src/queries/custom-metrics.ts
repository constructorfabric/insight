import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query";

import {
  createCustomMetric,
  deleteCustomMetric,
  getCustomMetric,
  listCustomMetrics,
  updateCustomMetric,
  type CreateCustomMetricRequest,
  type CustomMetric,
  type CustomMetricSummary,
  type UpdateCustomMetricRequest,
} from "@/api/metrics-client";
import {
  queryMetricResults,
  type MetricResultsResponse,
} from "@/api/metric-results-client";

const LIST_KEY = ["custom-metrics"] as const;

export function useCustomMetrics(): UseQueryResult<CustomMetricSummary[]> {
  return useQuery({
    queryKey: LIST_KEY,
    queryFn: listCustomMetrics,
    select: (data) => data.items,
  });
}

export function useCustomMetric(
  metricKey: string | null
): UseQueryResult<CustomMetric> {
  return useQuery({
    queryKey: ["custom-metrics", metricKey],
    queryFn: () => getCustomMetric(metricKey as string),
    enabled: metricKey !== null,
  });
}

export function useCreateCustomMetric() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateCustomMetricRequest) => createCustomMetric(body),
    onSuccess: () => client.invalidateQueries({ queryKey: LIST_KEY }),
  });
}

export function useUpdateCustomMetric(metricKey: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (body: UpdateCustomMetricRequest) =>
      updateCustomMetric(metricKey, body),
    onSuccess: (updated) => {
      client.invalidateQueries({ queryKey: LIST_KEY });
      client.setQueryData(["custom-metrics", metricKey], updated);
    },
  });
}

export function useDeleteCustomMetric() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (metricKey: string) => deleteCustomMetric(metricKey),
    onSuccess: () => client.invalidateQueries({ queryKey: LIST_KEY }),
  });
}

/** Parameters a preview needs beyond the metric key itself. */
export interface MetricPreviewInput {
  entityType: string;
  entityIds: string[];
  from: string;
  to: string;
}

/**
 * Preview a custom metric by running it through the shared results endpoint
 * with a single `period` view. A mutation (not a query) so the console fires
 * it on demand, the same way the query console runs a saved query.
 */
export function useCustomMetricPreview(metricKey: string) {
  return useMutation<MetricResultsResponse, Error, MetricPreviewInput>({
    mutationFn: (input: MetricPreviewInput) =>
      queryMetricResults({
        entity: {
          type: input.entityType as "person",
          ids: input.entityIds,
        },
        period: { from: input.from, to: input.to },
        metrics: [{ metric_key: metricKey, views: [{ view: "period" }] }],
      }),
  });
}
