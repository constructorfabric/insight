import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

/**
 * The mover's own word for how a sync ended, carried through unchanged.
 *
 * `unknown` is the service's addition: a recorded word outside the mover's
 * documented vocabulary. It is not a failure and not a success — a state the
 * page could not read.
 */
export type SyncStatus =
  | "pending"
  | "running"
  | "incomplete"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "unknown";

/**
 * One recorded sync.
 *
 * Every nullable field means "nobody measured this", never "this was zero" —
 * `records_reported: 0` is a sync that moved nothing, and `null` is a sync the
 * mover reported no count for at all.
 */
export interface SyncFact {
  job_id: string;
  status: SyncStatus;
  started_at: string | null;
  duration_ms: number | null;
  records_reported: number | null;
}

export interface ConnectorHealth {
  connector: string;
  configured: boolean;
  last_sync: SyncFact | null;
}

export interface ConnectorHealthSummary {
  /** When the service computed this answer. */
  as_of: string;
  /** When the mover was last read. Null before the first sweep sealed. */
  checked_at: string | null;
  /**
   * The median gap between the recent reads, measured from the record itself —
   * not a configured schedule. Null where too few reads are recorded.
   */
  typical_read_interval_ms: number | null;
  /** False when nothing has been recorded at all. */
  history_available: boolean;
  /** Already ordered by what needs acting on; the page does not re-sort. */
  connectors: ConnectorHealth[];
}

export interface ConnectorSyncHistory {
  connector: string;
  syncs: SyncFact[];
  /** The most rows this window can hold, so the page can say it is a window. */
  window: number;
}

export async function getConnectorHealth(): Promise<ConnectorHealthSummary> {
  const res = await fetchWithAuth(`${BASE}/connector-health`);
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as ConnectorHealthSummary;
}

export async function getConnectorSyncs(
  connector: string,
): Promise<ConnectorSyncHistory> {
  const res = await fetchWithAuth(
    `${BASE}/connector-health/${encodeURIComponent(connector)}/syncs`,
  );
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as ConnectorSyncHistory;
}
