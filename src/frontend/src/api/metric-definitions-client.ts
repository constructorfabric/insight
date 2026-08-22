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
  MetricEntityType,
  MetricFormat,
} from "@/api/metric-results-client";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export type MetricDefinitionSchemaStatus = "ok" | "error" | "unchecked";

export type MetricDefinitionOrigin = "builtin" | "custom";

export interface MetricDefinition {
  metric_key: string;
  entity_type: MetricEntityType;
  label: string;
  short_label: string | null;
  description: string | null;
  explanation: string | null;
  unit: string | null;
  format: MetricFormat;
  direction: MetricDirection;
  dimensions: string[];
  is_enabled: boolean;
  /**
   * `builtin` reads managed observation relations; `custom` executes inline
   * SQL at query time. The validator stamps `schema_status` and
   * `last_observed_date` from materialized relations only, so for `custom`
   * they stay "unchecked" / absent however much data the metric serves —
   * reading their absence as "never measured" is wrong for those.
   */
  origin: MetricDefinitionOrigin;
  schema_status: MetricDefinitionSchemaStatus;
  /** Why schema_status is "error"; null otherwise. */
  schema_error_code: MetricSchemaErrorCode | null;
  /** ISO date of the newest observation ever seen; null = no data yet. */
  last_observed_date: string | null;
  /**
   * How many days back from `last_observed_date` the suppliers may still
   * revise. Absent where nothing revises — never read absence as "revised
   * forever".
   */
  revision_window_days?: number | null;
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
