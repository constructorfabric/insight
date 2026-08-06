/**
 * Wire types + fetch wrapper for `GET /metric-definitions`.
 *
 * Display listing of the unified metric definitions: every definition
 * visible to the tenant, including disabled or schema-broken ones — the
 * listing doubles as a health surface, so availability is reported
 * (`is_enabled`, `schema_status`) rather than filtered. All human-facing
 * copy (label, description, explanation) is server-owned.
 */

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";
import type {
  MetricDrilldownCapability,
  MetricDirection,
  MetricFormat,
} from "@/api/metric-results-client";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export type MetricDefinitionSchemaStatus = "ok" | "error" | "unchecked";

export interface MetricDefinition {
  metric_key: string;
  label: string;
  short_label: string | null;
  description: string | null;
  explanation: string | null;
  unit: string | null;
  format: MetricFormat;
  direction: MetricDirection;
  dimensions: string[];
  is_enabled: boolean;
  schema_status: MetricDefinitionSchemaStatus;
  /** Why schema_status is "error"; null otherwise. */
  schema_error_code: MetricSchemaErrorCode | null;
  /** ISO date of the newest observation ever seen; null = no data yet. */
  last_observed_date: string | null;
  drilldown?: MetricDrilldownCapability;
}

type MetricSchemaErrorCode =
  | "table_not_found"
  | "column_not_found"
  | "dimension_not_covered"
  | "unknown";

export interface MetricDefinitionListResponse {
  metrics: MetricDefinition[];
}

export async function listMetricDefinitions(): Promise<MetricDefinitionListResponse> {
  const res = await fetchWithAuth(`${BASE}/metric-definitions`, {
    method: "GET",
  });
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
  try {
    return (await res.json()) as MetricDefinitionListResponse;
  } catch {
    throw new AnalyticsApiError(res.status, { error: "invalid_json" });
  }
}
