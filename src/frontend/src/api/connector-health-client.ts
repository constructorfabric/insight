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
  /** The mover's job this summary resolves to. */
  job_id: string | null;
  started_at: string;
  /**
   * Null until the mover's history has been swept — only it knows how long a
   * sync took and how much it moved, so a pipeline-only row reports neither
   * rather than reporting zeros nobody measured.
   */
  duration_ms: number | null;
  records_moved: number | null;
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
  /** When the response was assembled. Says nothing about the facts' age. */
  as_of: string;
  /**
   * When a controller tick last finished, or null when none ever has. The only
   * freshness the page may state — `as_of` is the reader's own clock and would
   * read as "just now" however long ago the controller last ran.
   */
  swept_at: string | null;
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
  /** The mover's job, so an event lines up with the summary by identity. */
  job_id: string | null;
  started_at: string;
  duration_ms: number;
  /** Null on a row no sweep has reached: nobody counted, which is not zero. */
  records_moved: number | null;
  rows_landed: number | null;
}

/** The trigger words the surface serves. Anything else is unknown provenance. */
export const SYNC_TRIGGERS: readonly SyncTrigger[] = [
  "claimed",
  "out_of_band",
  "unclaimed",
];

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
