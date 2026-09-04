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
  /**
   * ISO date of the oldest observation the metric currently holds; null = it
   * holds none, whether never observed or retained no longer. It moves forward
   * as retention drops old rows, so it is not the date collection began, and it
   * does not pair with `last_observed_date` as an interval of what is readable.
   */
  first_observed_date: string | null;
  /**
   * ISO date of the newest observation ever seen; null = none ever. A
   * high-water mark, never cleared.
   */
  last_observed_date: string | null;
  /**
   * ISO date of the newest delivered day that can no longer change. Absent
   * where nothing revises — never read absence as "revised forever".
   */
  settled_through?: string | null;
  /**
   * Legacy: a conservative day count standing in for `settled_through`, kept
   * for consumers written before it existed. It cannot express a boundary that
   * moves with the billing month, so it over-states there. New code reads
   * `settled_through`.
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
