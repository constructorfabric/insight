import type {
  ConnectorHealth,
  ConnectorHealthSummary,
  SyncFact,
  SyncStatus,
} from "@/api/connector-health-client";

/**
 * Every word this page says about a connector, and the one function that
 * decides which of them applies.
 *
 * The precedence lives here rather than spread across cells so that "what does
 * this row claim" is a question with one answer. The page reports recorded
 * facts and never a verdict the record cannot back: nothing here says a
 * connector is delivering, because the ledger holds the mover's own account of
 * its syncs and nothing that corroborates it.
 */
export type ConnectorTone = "failing" | "unknown" | "active" | "ok" | "idle";

export interface ConnectorStateView {
  /** Stable identifier, for tests and for the row's data attribute. */
  state:
    | "no_longer_configured"
    | "never_synced"
    | "syncing"
    | "sync_failed"
    | "sync_incomplete"
    | "sync_cancelled"
    | "state_unknown"
    | "sync_ok";
  /** What the row says. Never colour alone. */
  label: string;
  tone: ConnectorTone;
}

const STATE: Record<SyncStatus, ConnectorStateView> = {
  pending: { state: "syncing", label: "syncing", tone: "active" },
  running: { state: "syncing", label: "syncing", tone: "active" },
  failed: { state: "sync_failed", label: "sync failed", tone: "failing" },
  incomplete: {
    state: "sync_incomplete",
    label: "sync incomplete",
    tone: "failing",
  },
  cancelled: {
    state: "sync_cancelled",
    label: "sync cancelled",
    tone: "idle",
  },
  succeeded: { state: "sync_ok", label: "sync ok", tone: "ok" },
  unknown: { state: "state_unknown", label: "state unknown", tone: "unknown" },
};

const NO_LONGER_CONFIGURED: ConnectorStateView = {
  state: "no_longer_configured",
  label: "no longer configured",
  tone: "idle",
};

const NEVER_SYNCED: ConnectorStateView = {
  state: "never_synced",
  label: "never synced",
  tone: "idle",
};

/**
 * Configuration is read before the sync outcome, deliberately.
 *
 * A connector taken out of configuration is a decision, not a fault, and its
 * last sync having failed does not change that — reporting it as failing would
 * send an operator to fix something nobody asked for any more.
 */
export function describeConnector(row: ConnectorHealth): ConnectorStateView {
  if (!row.configured) return NO_LONGER_CONFIGURED;
  if (row.last_sync === null) return NEVER_SYNCED;
  return STATE[row.last_sync.status] ?? STATE.unknown;
}

/* ── what the page says about its own freshness ───────────────────────── */

/**
 * How many typical intervals may pass before the page stops presenting its
 * facts as current.
 *
 * One missed read is a hiccup; three in a row is a recorder that stopped. The
 * page cannot know the intended cadence — nothing on the read path does — so
 * the comparison is against the interval actually observed between recent
 * reads.
 */
export const STALE_AFTER_INTERVALS = 3;

export interface RecordingView {
  state: "never_read" | "stopped" | "current";
  /** The headline sentence. */
  label: string;
  /** The reassurance or the warning under it, empty when there is nothing to add. */
  detail: string;
}

export function describeRecording(
  summary: Pick<
    ConnectorHealthSummary,
    "as_of" | "checked_at" | "typical_read_interval_ms" | "history_available"
  >,
): RecordingView {
  if (!summary.history_available || summary.checked_at === null) {
    return {
      state: "never_read",
      label: "Nothing has been read from the connectors yet",
      detail:
        "This page fills in once the reconcile loop has read the data mover once.",
    };
  }

  const age = ageMs(summary.as_of, summary.checked_at);
  const interval = summary.typical_read_interval_ms;
  const stopped =
    age !== null &&
    interval !== null &&
    interval > 0 &&
    age > interval * STALE_AFTER_INTERVALS;

  if (stopped) {
    return {
      state: "stopped",
      label: `Last checked ${describeAge(age)} — recording appears to have stopped`,
      detail: "The connector states below may no longer be current.",
    };
  }

  return {
    state: "current",
    label:
      age === null
        ? "Last checked at an unreadable time"
        : `Last checked ${describeAge(age)}`,
    detail: "",
  };
}

function ageMs(asOf: string, checkedAt: string): number | null {
  const now = Date.parse(asOf);
  const then = Date.parse(checkedAt);
  if (Number.isNaN(now) || Number.isNaN(then)) return null;
  return Math.max(0, now - then);
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/** Coarse on purpose: the exact second of a periodic read is never the point. */
export function describeAge(ageInMs: number): string {
  if (ageInMs < MINUTE_MS) return "just now";
  if (ageInMs < HOUR_MS) return `${Math.floor(ageInMs / MINUTE_MS)} min ago`;
  if (ageInMs < DAY_MS) return `${Math.floor(ageInMs / HOUR_MS)} h ago`;
  return `${Math.floor(ageInMs / DAY_MS)} d ago`;
}

/* ── the cells ────────────────────────────────────────────────────────── */

/**
 * What an unmeasured value prints.
 *
 * Every absent number on this page renders as this rather than as a zero: the
 * two are different answers, and a zero where nobody measured would be the page
 * asserting something no one recorded.
 */
export const UNMEASURED = "—";

export function formatDuration(durationMs: number | null): string {
  if (durationMs === null) return UNMEASURED;
  if (durationMs < 1_000) return `${durationMs} ms`;
  const seconds = durationMs / 1_000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  if (minutes < 60) return `${minutes}m ${rest}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

export function formatRecords(records: number | null): string {
  return records === null ? UNMEASURED : records.toLocaleString("en-US");
}

export function formatStarted(startedAt: string | null): string {
  if (startedAt === null) return UNMEASURED;
  const parsed = Date.parse(startedAt);
  if (Number.isNaN(parsed)) return UNMEASURED;
  return new Date(parsed).toISOString().replace("T", " ").slice(0, 19) + "Z";
}

/** The status word for one sync in the expanded history. */
export function describeSync(sync: SyncFact): ConnectorStateView {
  return STATE[sync.status] ?? STATE.unknown;
}
