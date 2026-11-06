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

/**
 * Which installation of a connector a row is.
 *
 * One connector can be configured more than once — a second Secret naming its
 * own source id — so the name alone does not identify what synced. Both halves
 * are absent together on history recorded before the ledger carried the
 * identity that no single installation could be shown to own.
 */
export interface ConnectorInstance {
  tenant_id?: string | null;
  source_id?: string | null;
}

export type ScheduleOrigin = "api" | "annotation" | "descriptor" | "default";

export interface InheritedSchedule {
  crons: string[];
  origin: string;
}

export interface AppliedSchedule {
  crons: string[];
  suspended: boolean;
  timezone?: string | null;
  origin?: ScheduleOrigin | null;
}

export interface RecordedSchedule extends Omit<AppliedSchedule, "origin"> {
  origin: string;
  revision: number;
  observed_at: string;
}

export interface ScheduleAttemptError {
  attempted_revision: number;
  attempted_at: string;
  message: string;
}

export interface ConnectorHealth extends ConnectorInstance {
  connector: string;
  configured: boolean;
  schedule?: AppliedSchedule | null;
  schedule_request?: ScheduleRequestState | null;
  last_sync?: SyncFact | null;
}

export type ScheduleStatus = "inherited" | "pending" | "applied" | "error";

export interface RequestedSchedule {
  crons: string[];
  updated_at: string;
  updated_by: string;
}

export interface ScheduleRequestState {
  crons: string[];
  revision: number;
  status: ScheduleStatus;
  updated_at?: string | null;
  updated_by?: string | null;
  applied?: RecordedSchedule | null;
  inherited?: InheritedSchedule | null;
  error?: ScheduleAttemptError | null;
}

export interface ConnectorSchedule {
  connector: string;
  source_id: string;
  revision: number;
  requested?: RequestedSchedule | null;
  applied?: RecordedSchedule | null;
  inherited?: InheritedSchedule | null;
  error?: ScheduleAttemptError | null;
  status: ScheduleStatus;
}

export interface ScheduleRead {
  schedule: ConnectorSchedule;
  etag: string;
}

async function scheduleRequest(
  instance: ConnectorInstanceRef,
  method: "GET" | "PUT" | "DELETE",
  etag?: string,
  crons?: string[]
): Promise<ScheduleRead> {
  const headers: Record<string, string> = {};
  if (etag !== undefined) headers["If-Match"] = etag;
  if (method === "PUT") headers["Content-Type"] = "application/json";
  const res = await fetchWithAuth(
    `${BASE}/connectors/${encodeURIComponent(instance.connector)}/${encodeURIComponent(instance.source_id ?? "")}/schedule`,
    {
      method,
      headers,
      body: method === "PUT" ? JSON.stringify({ crons }) : undefined,
    }
  );
  if (!res.ok)
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  const schedule = (await res.json()) as ConnectorSchedule;
  const revision = res.headers.get("ETag");
  if (revision === null)
    throw new Error(
      "The schedule response has no revision. Reload and try again."
    );
  return { schedule, etag: revision };
}

export const getConnectorSchedule = (instance: ConnectorInstanceRef) =>
  scheduleRequest(instance, "GET");
export const putConnectorSchedule = (
  instance: ConnectorInstanceRef,
  etag: string,
  crons: string[]
) => scheduleRequest(instance, "PUT", etag, crons);
export const deleteConnectorSchedule = (
  instance: ConnectorInstanceRef,
  etag: string
) => scheduleRequest(instance, "DELETE", etag);

/** One installation, as the caller names it when asking for its history. */
export interface ConnectorInstanceRef extends ConnectorInstance {
  connector: string;
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

export interface ConnectorSyncHistory extends ConnectorInstance {
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
  instance: ConnectorInstanceRef
): Promise<ConnectorSyncHistory> {
  // Both halves or neither. The service refuses half an identity rather than
  // widening the window back to every installation, and a source id is unique
  // only within a tenant — so sending one alone would be asking a question
  // whose answer could not be trusted even if it were served.
  const scope = new URLSearchParams();
  if (instance.tenant_id != null && instance.source_id != null) {
    scope.set("tenant_id", instance.tenant_id);
    scope.set("source_id", instance.source_id);
  }
  const query = scope.toString();
  const res = await fetchWithAuth(
    `${BASE}/connector-health/${encodeURIComponent(instance.connector)}/syncs` +
      (query === "" ? "" : `?${query}`)
  );
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as ConnectorSyncHistory;
}
