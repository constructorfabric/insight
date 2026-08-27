/**
 * Wire types + fetch wrappers for the AI assistant API (`/v1/ai/*`).
 *
 * Two switches decide whether any of this is reachable: the deployment's own
 * (`getAiConfig`) and whether the caller has stored a key
 * (`getAiCredentialStatus`). The key itself is write-only — the server returns
 * its last four characters and nothing more.
 */

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

const JSON_HEADERS = { "Content-Type": "application/json" };

export type ContextScope = "tenant" | "person";

export interface AiConfig {
  enabled: boolean;
  model: string;
  /** The stand pays with its own key, so nobody stores one. */
  stand_key?: boolean;
  /** Only admins may ask for an explanation here. */
  admin_only?: boolean;
}

export interface AiCredentialStatus {
  configured: boolean;
  /** Last four characters of the stored key; empty when none is stored. */
  hint: string;
}

export interface AiSettings {
  system_prompt: string;
  is_default: boolean;
}

export interface ContextEntry {
  id: string;
  scope: ContextScope;
  title: string;
  body: string;
  updated_at: string;
}

export interface ContextListResponse {
  items: ContextEntry[];
}

export interface CreateContextRequest {
  scope: ContextScope;
  title: string;
  body: string;
}

export interface UpdateContextRequest {
  title?: string;
  body?: string;
}

export type SnapshotScope = "person" | "organisation";

/** One line of a chart, as it is drawn. */
export interface SnapshotSeries {
  label: string;
  /** Readings per bucket, oldest first; a gap is null. */
  points: (number | null)[];
}

/** The reading as it is on screen — what the model is asked to explain. */
export interface MetricSnapshot {
  metric_key: string;
  label: string;
  value: string;
  period: string;
  since: string;
  until: string;
  delta: string;
  peer: string;
  help: string;
  trend: (number | null)[];
  scope?: SnapshotScope;
  /** Bucket start dates the series are indexed by, oldest first. */
  bucket_starts?: string[];
  /** The chart's lines, when the reading is a chart rather than a tile. */
  series?: SnapshotSeries[];
}

export interface Explanation {
  text: string;
  model: string;
  tenant_context_entries: number;
  person_context_entries: number;
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

async function expectNoContent(res: Response): Promise<void> {
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
}

export async function getAiConfig(): Promise<AiConfig> {
  const res = await fetchWithAuth(`${BASE}/ai/config`, { method: "GET" });
  return parseOk<AiConfig>(res);
}

export async function getAiCredentialStatus(): Promise<AiCredentialStatus> {
  const res = await fetchWithAuth(`${BASE}/ai/credentials`, { method: "GET" });
  return parseOk<AiCredentialStatus>(res);
}

export async function putAiCredential(
  token: string
): Promise<AiCredentialStatus> {
  const res = await fetchWithAuth(`${BASE}/ai/credentials`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ token }),
  });
  return parseOk<AiCredentialStatus>(res);
}

export async function deleteAiCredential(): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/ai/credentials`, {
    method: "DELETE",
  });
  return expectNoContent(res);
}

export async function getAiSettings(): Promise<AiSettings> {
  const res = await fetchWithAuth(`${BASE}/ai/settings`, { method: "GET" });
  return parseOk<AiSettings>(res);
}

export async function putAiSettings(systemPrompt: string): Promise<AiSettings> {
  const res = await fetchWithAuth(`${BASE}/ai/settings`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ system_prompt: systemPrompt }),
  });
  return parseOk<AiSettings>(res);
}

export async function resetAiSettings(): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/ai/settings`, { method: "DELETE" });
  return expectNoContent(res);
}

export async function listAiContext(): Promise<ContextListResponse> {
  const res = await fetchWithAuth(`${BASE}/ai/context`, { method: "GET" });
  return parseOk<ContextListResponse>(res);
}

export async function createAiContext(
  body: CreateContextRequest
): Promise<ContextEntry> {
  const res = await fetchWithAuth(`${BASE}/ai/context`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<ContextEntry>(res);
}

export async function updateAiContext(
  id: string,
  body: UpdateContextRequest
): Promise<ContextEntry> {
  const res = await fetchWithAuth(`${BASE}/ai/context/${id}`, {
    method: "PATCH",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  return parseOk<ContextEntry>(res);
}

export async function deleteAiContext(id: string): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/ai/context/${id}`, {
    method: "DELETE",
  });
  return expectNoContent(res);
}

export async function explainMetric(
  snapshot: MetricSnapshot
): Promise<Explanation> {
  const res = await fetchWithAuth(`${BASE}/ai/explain`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(snapshot),
  });
  return parseOk<Explanation>(res);
}
