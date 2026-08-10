/**
 * Wire types + fetch wrappers for the custom-metric authoring API
 * (`/v1/metrics`).
 *
 * A custom metric is a tenant-owned metric graph: display metadata plus a
 * custom `observation_sql` that emits the observation contract, the measures
 * that SQL produces, the dimensions it allows, and the input wiring the
 * computation consumes. CRUD is plain metadata; previewing a metric reaches
 * ClickHouse through the shared `/v1/metric-results` endpoint (see
 * `metric-results-client`). The `{tenant}` is injected server-side from the
 * session — the console never sends a tenant.
 */

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";
import type {
  MetricComputation,
  MetricDirection,
  MetricFormat,
} from "@/api/metric-results-client";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export type { MetricComputation, MetricDirection, MetricFormat };

/** Role a measure plays in the computation. */
export type MetricInputRole = "value" | "numerator" | "denominator";

export interface MetricInput {
  role: MetricInputRole;
  measure_key: string;
}

export interface MetricTransform {
  multiplier?: number | null;
  offset?: number | null;
  clamp_min?: number | null;
  clamp_max?: number | null;
}

/** List item — no graph body. */
export interface CustomMetricSummary {
  metric_key: string;
  label: string;
  computation: MetricComputation;
  entity_type: string;
}

/**
 * The full custom-metric graph, without provenance. This is the create/update
 * request shape and the unit of export/import.
 */
export interface CustomMetricGraph {
  /** Dotted "family.name". */
  metric_key: string;
  label: string;
  short_label?: string | null;
  description?: string | null;
  explanation?: string | null;
  /** e.g. "person". */
  entity_type: string;
  unit?: string | null;
  format: MetricFormat;
  direction: MetricDirection;
  computation: MetricComputation;
  /** Required iff `computation === "ratio"`. */
  scale?: number | null;
  peer_cohort_key?: string | null;
  transform?: MetricTransform | null;
  /** `^[a-z][a-z0-9_]*$`. */
  source_key: string;
  /** Custom SQL emitting the observation contract. */
  observation_sql: string;
  /** measure_keys the SQL emits. */
  measures: string[];
  /** allowed dimension keys. */
  dimensions: string[];
  inputs: MetricInput[];
}

/** A stored custom metric, as returned by fetch/create/update — carries its
 *  provenance. */
export interface CustomMetric extends CustomMetricGraph {
  origin: "custom";
}

export type CreateCustomMetricRequest = CustomMetricGraph;
export type UpdateCustomMetricRequest = CustomMetricGraph;

export interface CustomMetricListResponse {
  items: CustomMetricSummary[];
}

export interface CustomMetricExportResponse {
  metrics: CustomMetricGraph[];
}

export interface CustomMetricImportRequest {
  metrics: CustomMetricGraph[];
}

export interface CustomMetricImportResponse {
  imported: number;
  /** metric_keys that were not imported (e.g. already present). */
  skipped: string[];
}

async function parseOk<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
  try {
    return (await res.json()) as T;
  } catch {
    throw new AnalyticsApiError(res.status, { error: "invalid_json" });
  }
}

const JSON_HEADERS = { "Content-Type": "application/json" };

export async function listCustomMetrics(): Promise<CustomMetricListResponse> {
  const res = await fetchWithAuth(`${BASE}/metrics`, { method: "GET" });
  return parseOk<CustomMetricListResponse>(res);
}

export async function getCustomMetric(
  metricKey: string
): Promise<CustomMetric> {
  const res = await fetchWithAuth(
    `${BASE}/metrics/${encodeURIComponent(metricKey)}`,
    { method: "GET" }
  );
  return parseOk<CustomMetric>(res);
}

export async function createCustomMetric(
  body: CreateCustomMetricRequest
): Promise<CustomMetric> {
  const res = await fetchWithAuth(`${BASE}/metrics`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<CustomMetric>(res);
}

export async function updateCustomMetric(
  metricKey: string,
  body: UpdateCustomMetricRequest
): Promise<CustomMetric> {
  const res = await fetchWithAuth(
    `${BASE}/metrics/${encodeURIComponent(metricKey)}`,
    {
      method: "PUT",
      headers: JSON_HEADERS,
      body: JSON.stringify(body),
    }
  );
  return parseOk<CustomMetric>(res);
}

export async function deleteCustomMetric(metricKey: string): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/metrics/${encodeURIComponent(metricKey)}`,
    { method: "DELETE" }
  );
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
}

export async function exportCustomMetrics(): Promise<CustomMetricExportResponse> {
  const res = await fetchWithAuth(`${BASE}/metrics/export`, { method: "GET" });
  return parseOk<CustomMetricExportResponse>(res);
}

export async function importCustomMetrics(
  body: CustomMetricImportRequest
): Promise<CustomMetricImportResponse> {
  const res = await fetchWithAuth(`${BASE}/metrics/import`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<CustomMetricImportResponse>(res);
}
