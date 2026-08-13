import { useQuery } from "@tanstack/react-query";

import { AnalyticsApiError } from "@/api/analytics-client";
import {
  queryMetricDrilldown,
  type MetricEvidenceSelection,
} from "@/api/metric-drilldown-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";

/**
 * How many rows a section's inline detail asks for.
 *
 * Enough to show a period's shape at day grain without paging, and far short
 * of what the evidence dialog is for. A section is a first look, not an
 * export.
 */
export const DETAIL_LIMIT = 200;

/**
 * The rows behind one metric, for showing inline rather than in a dialog.
 *
 * Shares the dialog's query key so the two never fetch the same page twice:
 * opening the full evidence for a metric already shown in a section starts
 * from cache.
 */
export function useMetricDetail(
  selection: MetricEvidenceSelection | null | undefined,
  enabled = true
) {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  return useQuery({
    // The dialog keys on the same shape with its own page size, so the two
    // stay distinct entries rather than one clobbering the other's rows.
    queryKey: ["metric-drilldown", sessionScope, selection, DETAIL_LIMIT],
    queryFn: ({ signal }) => {
      if (!selection) throw new Error("Metric detail selection is missing");
      return queryMetricDrilldown(
        { ...selection, limit: DETAIL_LIMIT },
        signal
      );
    },
    enabled: enabled && sessionScope != null && selection != null,
    retry: (failureCount, error) =>
      failureCount < 1 &&
      (!(error instanceof AnalyticsApiError) || error.status >= 500),
  });
}
