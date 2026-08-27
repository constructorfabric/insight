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
 *
 * Every function here is total over what the contract permits — which includes
 * an absent key for any nullable field, and a status word this build has never
 * heard of. A partial function would take the whole portal to its error
 * boundary on a legal response.
 */
export type ConnectorTone = "failing" | "unknown" | "active" | "ok" | "idle";

export type ConnectorStateName =
  | "no_longer_configured"
  | "never_synced"
  | "queued"
  | "syncing"
  | "sync_failed"
  | "sync_incomplete"
  | "sync_cancelled"
  | "state_unknown"
  | "sync_ok";

export interface ConnectorStateView {
  /** Stable identifier, for tests and for the row's data attribute. */
  state: ConnectorStateName;
  /** What the row says. Never colour alone. */
  label: string;
  tone: ConnectorTone;
}

/**
 * One entry per word the mover uses, kept apart.
 *
 * `pending` and `running` are not merged. A queued job has no start time, so
 * merging them makes the row say "syncing" beside a start of "—" — and erases
 * the only signal that separates "queued and not picked up" from "running now",
 * which is one of the states an operator opens this page for.
 */
const STATE: Record<SyncStatus, ConnectorStateView> = {
  pending: { state: "queued", label: "queued", tone: "active" },
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

/** A word outside this build's vocabulary is a state it cannot read. */
function stateOf(status: string | undefined): ConnectorStateView {
  return STATE[status as SyncStatus] ?? STATE.unknown;
}

/**
 * Configuration is read before the sync outcome, deliberately.
 *
 * A connector taken out of configuration is a decision, not a fault, and its
 * last sync having failed does not change that — reporting it as failing would
 * send an operator to fix something nobody asked for any more.
 */
export function describeConnector(row: ConnectorHealth): ConnectorStateView {
  if (!row.configured) return NO_LONGER_CONFIGURED;
  if (row.last_sync == null) return NEVER_SYNCED;
  return stateOf(row.last_sync.status);
}

/** The status word for one sync in the expanded history. */
export function describeSync(sync: SyncFact): ConnectorStateView {
  return stateOf(sync.status);
}

/* ── what the page says about its own freshness ───────────────────────── */

/**
 * How many typical intervals may pass before the page stops presenting its
 * facts as current.
 *
 * One missed read is a hiccup; three in a row is a recorder that stopped.
 */
export const STALE_AFTER_INTERVALS = 3;

/**
 * The band a measured interval is clamped into before it is multiplied.
 *
 * Both ends are load-bearing. Without a floor, a burst of closely spaced
 * sweeps — a restart loop, a manual tick — drags the median down and the page
 * then reports a live install as stopped. Without a ceiling, a long measured
 * interval would let a genuinely stopped recorder sit unreported for days.
 */
const MIN_INTERVAL_MS = 5 * 60_000;
const MAX_INTERVAL_MS = 60 * 60_000;

export interface RecordingView {
  state: "never_read" | "stopped" | "unmeasured" | "current" | "unreadable";
  /** The headline sentence. */
  label: string;
  /** The warning or reassurance under it; empty when there is nothing to add. */
  detail: string;
}

export function describeRecording(
  summary: Pick<
    ConnectorHealthSummary,
    "as_of" | "checked_at" | "typical_read_interval_ms" | "history_available"
  >,
): RecordingView {
  if (!summary.history_available || summary.checked_at == null) {
    return {
      state: "never_read",
      label: "Nothing has been read from the connectors yet",
      detail:
        "This page fills in once the reconcile loop has read the data mover once.",
    };
  }

  const age = ageMs(summary.as_of, summary.checked_at);
  if (age === null) {
    return {
      state: "unreadable",
      label: "Cannot tell when the connectors were last read",
      detail: "Nothing below can be dated, so treat it as unverified.",
    };
  }

  const threshold = staleAfter(summary.typical_read_interval_ms);

  // No measured interval, so there is no cadence to be late against. The page
  // states the age and says the cadence is unknown — rather than inventing a
  // threshold and asserting that recording stopped, which is a conclusion
  // nothing in the record supports.
  if (threshold === null) {
    return {
      state: "unmeasured",
      label: `Last checked ${describeAge(age)}`,
      detail:
        "Too few reads are recorded to know how often this should happen, so " +
        "nothing here says whether that is normal.",
    };
  }

  if (age > threshold) {
    return {
      state: "stopped",
      label: `Last checked ${describeAge(age)} — recording appears to have stopped`,
      detail: "The connector states below may no longer be current.",
    };
  }

  return { state: "current", label: `Last checked ${describeAge(age)}`, detail: "" };
}

/**
 * How old a read may be before the page stops presenting its facts as current,
 * or null where the record cannot say.
 *
 * The interval is clamped into a band first. Without a floor, a burst of
 * closely spaced sweeps — a restart loop, a manual tick — drags the median down
 * and a live install then reads as stopped. Without a ceiling, one unusually
 * long gap would let a genuinely stopped recorder sit unreported for days.
 */
function staleAfter(interval: number | null | undefined): number | null {
  if (interval == null || !Number.isFinite(interval) || interval <= 0) {
    return null;
  }
  const clamped = Math.min(Math.max(interval, MIN_INTERVAL_MS), MAX_INTERVAL_MS);
  return clamped * STALE_AFTER_INTERVALS;
}

/**
 * How long ago the mover was read, or null where the pair cannot say.
 *
 * A negative age is not clamped to zero: the two stamps come from clocks that
 * are supposed to agree, and one ahead of the other means neither can date the
 * page. Reporting "just now" there would be the page asserting a freshness it
 * has no basis for.
 */
function ageMs(asOf: string, checkedAt: string): number | null {
  const now = parseStamp(asOf);
  const then = parseStamp(checkedAt);
  if (now === null || then === null) return null;
  const age = now - then;
  return age >= 0 ? age : null;
}

/**
 * `Date.parse` alone is far too lenient to be a guard: it reads `"2026"` as a
 * date and `"0"` as one too, so a truncated or garbage stamp would render as a
 * confident absolute timestamp. The service emits RFC 3339, so requiring that
 * shape costs nothing and refuses everything else.
 */
const RFC3339 = /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:?\d{2})?$/;

function parseStamp(raw: string | null | undefined): number | null {
  if (typeof raw !== "string" || !RFC3339.test(raw)) return null;
  const parsed = Date.parse(raw);
  return Number.isNaN(parsed) ? null : parsed;
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

/**
 * A duration, or absence.
 *
 * Rounding is done once, at the top, so no unit can carry 60 of the one below
 * it — `1m 60s` and `60.0 s` are not durations. A negative value is malformed
 * rather than measured, and prints as absence.
 */
export function formatDuration(durationMs: number | null | undefined): string {
  if (durationMs == null || !Number.isFinite(durationMs) || durationMs < 0) {
    return UNMEASURED;
  }
  if (durationMs < 1_000) return `${Math.round(durationMs)} ms`;

  const totalSeconds = Math.round(durationMs / 1_000);
  if (totalSeconds < 60) return `${(durationMs / 1_000).toFixed(1)} s`;

  const totalMinutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (totalMinutes < 60) return `${totalMinutes}m ${seconds}s`;
  return `${Math.floor(totalMinutes / 60)}h ${totalMinutes % 60}m`;
}

export function formatRecords(records: number | null | undefined): string {
  if (records == null || !Number.isFinite(records) || records < 0) {
    return UNMEASURED;
  }
  return records.toLocaleString("en-US");
}

export function formatStarted(startedAt: string | null | undefined): string {
  const parsed = parseStamp(startedAt);
  if (parsed === null) return UNMEASURED;
  return `${new Date(parsed).toISOString().replace("T", " ").slice(0, 19)}Z`;
}
