/**
 * Wire types + fetch wrappers for the query contract (`POST /v1/query`,
 * `GET /v1/datasets`). Shapes mirror the analytics OpenAPI document; the
 * tagged unions carry exactly the operands their variant takes.
 */

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export type QueryScalar = string | number | boolean;

export type QueryFilter =
  | { op: "eq"; field: string; value: QueryScalar }
  | { op: "in"; field: string; values: QueryScalar[] }
  | { op: "gt" | "gte" | "lt" | "lte"; field: string; value: QueryScalar }
  | { op: "between"; field: string; low: QueryScalar; high: QueryScalar }
  | { op: "not_null"; field: string }
  | { op: "like"; field: string; value: string }
  | { op: "match"; field: string; value: string };

export type QueryFilterOp = QueryFilter["op"];

export type QueryGroupAxis =
  | { axis: "dimension"; field: string }
  | { axis: "time" };

export type QueryFoldFn = "sum" | "avg" | "min" | "max";

export type QueryAggregate =
  | { fn: "count"; name: string; filter?: QueryFilter }
  | { fn: QueryFoldFn; name: string; field: string; filter?: QueryFilter };

export type QueryGrain = "day" | "week" | "month";

export interface QueryTime {
  field?: string;
  from: string;
  to: string;
  grain?: QueryGrain;
}

export type QueryDirection = "asc" | "desc";

export interface QueryOrder {
  by: string;
  dir?: QueryDirection;
}

export interface Query {
  dataset: string;
  filters?: QueryFilter[];
  group_by?: QueryGroupAxis[];
  aggregates: QueryAggregate[];
  time: QueryTime;
  order?: QueryOrder[];
  limit?: number;
}

export type QueryColumnKind = "dimension" | "label" | "bucket" | "aggregate";
export type QueryColumnType = "text" | "number" | "date";

export interface QueryAnswerColumn {
  name: string;
  kind: QueryColumnKind;
  type: QueryColumnType;
}

export interface QueryAnswer {
  columns: QueryAnswerColumn[];
  rows: unknown[][];
  flags: { truncated: boolean };
}

export interface QueryDatasetDimension {
  field: string;
  absent_value?: string | null;
  label?: string | null;
}

export interface QueryDatasetMeasurable {
  field: string;
}

export interface QueryDatasetTimeField {
  field: string;
  default: boolean;
}

export interface QueryLimits {
  default_limit: number;
  max_limit: number;
  max_filters: number;
  max_filter_values: number;
  max_aggregates: number;
  max_group_axes: number;
  max_order_terms: number;
}

export interface QueryDataset {
  key: string;
  time_fields: QueryDatasetTimeField[];
  dimensions: QueryDatasetDimension[];
  measurables: QueryDatasetMeasurable[];
  limits: QueryLimits;
}

export interface QueryDatasetList {
  datasets: QueryDataset[];
}

/** One entry of a refusal's `context.field_violations`. */
export interface QueryFieldViolation {
  field: string;
  description: string;
  reason: string;
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

export async function listDatasets(): Promise<QueryDatasetList> {
  const res = await fetchWithAuth(`${BASE}/datasets`, { method: "GET" });
  return parseOk<QueryDatasetList>(res);
}

export async function runQuery(body: Query): Promise<QueryAnswer> {
  const res = await fetchWithAuth(`${BASE}/query`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return parseOk<QueryAnswer>(res);
}

/**
 * Every field violation a refusal carries, so a form can mark each offending
 * input rather than only the first. Empty for anything but a field-named
 * `400`.
 */
export function queryFieldViolations(error: unknown): QueryFieldViolation[] {
  if (!(error instanceof AnalyticsApiError)) return [];
  const body = error.body;
  if (typeof body !== "object" || body === null) return [];
  const context = (body as { context?: unknown }).context;
  if (typeof context !== "object" || context === null) return [];
  const violations = (context as { field_violations?: unknown })
    .field_violations;
  if (!Array.isArray(violations)) return [];
  return violations.flatMap((entry) => {
    if (typeof entry !== "object" || entry === null) return [];
    const { field, description, reason } = entry as Record<string, unknown>;
    if (typeof field !== "string" || typeof description !== "string") return [];
    return [{ field, description, reason: typeof reason === "string" ? reason : "" }];
  });
}
