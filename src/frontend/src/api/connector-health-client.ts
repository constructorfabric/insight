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
 * `records_reported: 0` is a sync that moved nothing, and an absent value is a
 * sync the mover reported no count for at all.
 *
 * Absent AND nullable, deliberately. The generated contract lists only
 * `job_id` and `status` as required, following this service's convention for
 * every DTO, so a contract-legal response may omit the key entirely. Typing
 * these as `T | null` reads as safer and is not: `tsc` then lets a caller
 * assume the key is present, and the value that arrives is `undefined`.
 */
export interface SyncFact {
  job_id: string;
  /** The contract types this as a bare string, so a word this union does not
   * carry is a legal response — the mover's vocabulary can grow. */
  status: SyncStatus | string;
  started_at?: string | null;
  duration_ms?: number | null;
  records_reported?: number | null;
}

export interface ConnectorHealth {
  connector: string;
  configured: boolean;
  last_sync?: SyncFact | null;
}

export interface ConnectorHealthSummary {
  /** When the service computed this answer. */
  as_of: string;
  /** When the mover was last read. Absent before the first sweep sealed. */
  checked_at?: string | null;
  /**
   * The median gap between the recent reads, measured from the record itself —
   * not a configured schedule. Absent where too few reads are recorded.
   */
  typical_read_interval_ms?: number | null;
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
