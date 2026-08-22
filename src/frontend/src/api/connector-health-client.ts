/** Wire types + fetch wrapper for `GET /connector-health`. */

import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export interface ConnectorRow {
  connector: string;
  namespace: string;
  streams: number;
  streams_with_data: number;
  /**
   * Physical rows across active parts: on a deduplicating engine this sizes a
   * stream and does not count entities.
   */
  rows: number;
  /** ISO timestamp of the newest arrival; null = never delivered. */
  last_write: string | null;
}

export interface ConnectorHealthResponse {
  as_of: string;
  connectors: ConnectorRow[];
}

export async function getConnectorHealth(): Promise<ConnectorHealthResponse> {
  const res = await fetchWithAuth(`${BASE}/connector-health`, {
    method: "GET",
  });
  if (!res.ok) {
    const errorBody = await res.json().catch(() => null);
    throw new AnalyticsApiError(res.status, errorBody);
  }
  try {
    return (await res.json()) as ConnectorHealthResponse;
  } catch {
    throw new AnalyticsApiError(res.status, { error: "invalid_json" });
  }
}
