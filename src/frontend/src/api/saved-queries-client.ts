/**
 * Wire types + fetch wrappers for the saved-query API (`/v1/queries`).
 *
 * A saved query is a single `SELECT`/`WITH` over the read-only contract,
 * tenant-scoped. CRUD is plain metadata; only `/run` reaches ClickHouse,
 * executing read-only and returning untyped JSON rows (per-query dynamic
 * columns). The `{tenant}` parameter is injected server-side from the session —
 * the console never sends a tenant.
 */

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

/** List item — no `sql` body. */
export interface SavedQuerySummary {
  id: string;
  name: string;
  description?: string | null;
}

/** Full saved query, as returned by fetch/create/update. */
export interface SavedQuery {
  id: string;
  insight_tenant_id: string;
  name: string;
  description?: string | null;
  sql: string;
  created_at: string;
  updated_at: string;
}

export interface SavedQueryListResponse {
  items: SavedQuerySummary[];
}

export interface CreateSavedQueryRequest {
  name: string;
  description?: string | null;
  sql: string;
}

export interface UpdateSavedQueryRequest {
  name?: string;
  description?: string | null;
  sql?: string;
}

/** Optional run parameters; `{tenant}` is server-injected, never sent here. */
export interface RunSavedQueryRequest {
  period?: string;
}

/** A run result row: dynamic per-query columns, so untyped. */
export type RunResultRow = Record<string, unknown>;

export interface RunResponse {
  rows: RunResultRow[];
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

export async function listSavedQueries(): Promise<SavedQueryListResponse> {
  const res = await fetchWithAuth(`${BASE}/queries`, { method: "GET" });
  return parseOk<SavedQueryListResponse>(res);
}

export async function getSavedQuery(id: string): Promise<SavedQuery> {
  const res = await fetchWithAuth(`${BASE}/queries/${id}`, { method: "GET" });
  return parseOk<SavedQuery>(res);
}

export async function createSavedQuery(
  body: CreateSavedQueryRequest
): Promise<SavedQuery> {
  const res = await fetchWithAuth(`${BASE}/queries`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<SavedQuery>(res);
}

export async function updateSavedQuery(
  id: string,
  body: UpdateSavedQueryRequest
): Promise<SavedQuery> {
  const res = await fetchWithAuth(`${BASE}/queries/${id}`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<SavedQuery>(res);
}

export async function deleteSavedQuery(id: string): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/queries/${id}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
}

export async function runSavedQuery(
  id: string,
  body: RunSavedQueryRequest = {}
): Promise<RunResponse> {
  const res = await fetchWithAuth(`${BASE}/queries/${id}/run`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<RunResponse>(res);
}
