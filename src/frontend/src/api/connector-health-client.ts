import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

/**
 * How a sync was started, as the writers recorded it.
 *
 * `unclaimed` is unknown provenance — no run claimed the job and nothing
 * survived to corroborate against. It is never rendered as a manual sync.
 */
export type SyncTrigger = "claimed" | "out_of_band" | "unclaimed";

export interface RunFacts {
  status: string;
  /** The step the run reached; absent when it did not fail. */
  step: string | null;
  started_at: string;
  duration_ms: number;
  /** Outcome of this run's own transform step, when it got that far. */
  transform_status: string | null;
}

export interface SyncFacts {
  trigger: SyncTrigger;
  status: string;
  started_at: string;
  duration_ms: number;
  records_moved: number;
  /**
   * Rows measured as delivered by this sync, or null where nothing measured it.
   *
   * INVARIANT: null is absence, never zero delivery. A measured zero beside
   * moved records is the "reported records, storage gained none" finding; a
   * null means the question was not asked.
   */
  rows_landed: number | null;
}

export interface StorageFacts {
  observed_at: string;
  streams: number;
  streams_with_data: number;
  /**
   * Physical rows present when observed. On a deduplicating engine this sizes a
   * connector; it does not count entities.
   */
  physical_rows: number;
  bytes_on_disk: number;
}

export interface StreamFacts {
  stream: string;
  physical_rows: number;
  bytes_on_disk: number;
}

export interface ConnectorHealthRow {
  connector: string;
  /** Present in the newest recorded snapshot of the configured set. */
  configured: boolean;
  last_run: RunFacts | null;
  last_sync: SyncFacts | null;
  storage: StorageFacts | null;
  streams: StreamFacts[];
}

export interface ConnectorHealthResponse {
  as_of: string;
  /**
   * False when nothing has recorded a run yet. The page says so rather than
   * showing an empty table that reads as health.
   */
  history_available: boolean;
  connectors: ConnectorHealthRow[];
}

export interface RunEvent {
  event: string;
  status: string;
  step: string | null;
  /** Which writer recorded the row: `pipeline` or `sweep`. Not the trigger. */
  origin: string;
  trigger: SyncTrigger | null;
  started_at: string;
  duration_ms: number;
  records_moved: number;
  rows_landed: number | null;
}

export interface ConnectorRunsResponse {
  connector: string;
  runs: RunEvent[];
}

async function readJson<T>(path: string): Promise<T> {
  const res = await fetchWithAuth(`${BASE}${path}`);
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as T;
}

export async function getConnectorHealth(): Promise<ConnectorHealthResponse> {
  return readJson<ConnectorHealthResponse>("/connector-health");
}

export async function getConnectorRuns(
  connector: string
): Promise<ConnectorRunsResponse> {
  return readJson<ConnectorRunsResponse>(
    `/connector-health/${encodeURIComponent(connector)}/runs`
  );
}
