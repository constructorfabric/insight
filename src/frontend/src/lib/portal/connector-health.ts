import type {
  ConnectorHealthRow,
  SyncTrigger,
} from "@/api/connector-health-client";

/**
 * What the page says about a connector, composed from recorded facts.
 *
 * The server ships facts and no verdict, so the precedence lives here — in one
 * documented function rather than scattered across cells. Every state names an
 * observation, never a guess: nothing here asserts freshness, because the
 * declared thresholds have no runtime source.
 *
 * INVARIANT: a state is only reached from facts that are KNOWN. An absent
 * counter or an unmeasured delivery is a gap, and a gap must never be presented
 * as a finding — nor may an unrecognised status fall through to "delivering".
 */
export type ConnectorState =
  /** Reported moving records while storage gained none. Both halves measured. */
  | "misdelivered"
  /** The last run failed. */
  | "run_failed"
  /** The mover's own sync failed. */
  | "sync_failed"
  /** The sync succeeded and this run's transform did not. */
  | "transform_failed"
  /** A run or sync is still in flight — not yet an outcome. */
  | "in_flight"
  /** Nothing manages this connector any more, though it has history. */
  | "unmanaged"
  /** The newest sync ran outside the pipeline, so no transform followed it. */
  | "sync_without_transform"
  /** Runs complete and nothing has ever been stored. */
  | "nothing_stored"
  /** A recorded status this build has no reading for. Never "delivering". */
  | "state_unknown"
  /** The last run completed and nothing contradicts it. */
  | "delivering"
  /** Configured, and nothing has run yet. */
  | "never_ran"
  /** A schema exists, nothing configured it, nothing ran. */
  | "not_configured";

export interface ConnectorStateLabel {
  state: ConnectorState;
  /** What the operator reads. Words carry the state, so colour is not alone. */
  label: string;
  tone: "critical" | "warning" | "ok" | "idle";
}

/**
 * Every state, in attention order: what needs acting on first.
 *
 * Exported as the single ordering, so the tiles cannot drift from the
 * precedence below. Typed as a full record rather than an array so a new state
 * fails the build instead of silently never rendering a tile.
 */
const STATES: Record<ConnectorState, Omit<ConnectorStateLabel, "state"> & { tile: string }> = {
  misdelivered: {
    label: "recorded, nothing landed",
    tile: "nothing landed",
    tone: "critical",
  },
  run_failed: { label: "run failed", tile: "run failed", tone: "critical" },
  sync_failed: { label: "sync failed", tile: "sync failed", tone: "critical" },
  transform_failed: {
    label: "transform failed",
    tile: "transform failed",
    tone: "warning",
  },
  in_flight: { label: "in flight", tile: "in flight", tone: "warning" },
  unmanaged: {
    label: "no longer configured",
    tile: "no longer configured",
    tone: "warning",
  },
  sync_without_transform: {
    label: "sync without transform",
    tile: "sync without transform",
    tone: "warning",
  },
  nothing_stored: {
    label: "nothing stored",
    tile: "nothing stored",
    tone: "warning",
  },
  state_unknown: {
    label: "unrecognised state",
    tile: "unrecognised state",
    tone: "warning",
  },
  delivering: { label: "delivering", tile: "delivering", tone: "ok" },
  never_ran: { label: "never ran", tile: "never ran", tone: "idle" },
  not_configured: {
    label: "not configured",
    tile: "not configured",
    tone: "idle",
  },
};

export const STATE_ORDER = Object.keys(STATES) as ConnectorState[];

export function stateTileLabel(state: ConnectorState): string {
  return STATES[state].tile;
}

const TERMINAL_FAILURES = new Set(["failed", "cancelled"]);
const IN_FLIGHT = new Set(["running", "pending"]);
const SUCCEEDED = new Set(["ok"]);

/**
 * Every status word this build knows how to read.
 *
 * A word outside it is not evidence of health: the recorder's vocabulary can
 * outgrow the reader's, and falling through to "delivering" would turn a state
 * nobody here understands into a reassurance.
 */
function recognised(status: string | null | undefined): boolean {
  if (status === null || status === undefined) return true;
  return TERMINAL_FAILURES.has(status) || IN_FLIGHT.has(status) || SUCCEEDED.has(status);
}

function failed(status: string | null | undefined): boolean {
  return status !== null && status !== undefined && TERMINAL_FAILURES.has(status);
}

function inFlight(status: string | null | undefined): boolean {
  return status !== null && status !== undefined && IN_FLIGHT.has(status);
}

function isNewer(a: string, b: string): boolean {
  return new Date(a).getTime() > new Date(b).getTime();
}

/**
 * The one place a state is decided.
 *
 * Order matters and is deliberate: a delivery mismatch outranks a failed run,
 * because a run that reports success while nothing landed is the more
 * misleading of the two. `in_flight` outranks the quiet states because a sync
 * still running is not yet an outcome, and calling it delivering would state
 * something nobody knows.
 */
export function connectorState(row: ConnectorHealthRow): ConnectorState {
  const { last_run: run, last_sync: sync, storage } = row;

  if (!row.configured && !run && !sync) return "not_configured";

  // Both halves must be known: an unrecorded counter or an unmeasured delivery
  // is a gap, and a gap is not a finding.
  if (sync && sync.records_moved !== null && sync.records_moved > 0 && sync.rows_landed === 0) {
    return "misdelivered";
  }

  if (failed(run?.status)) return "run_failed";
  if (failed(sync?.status)) return "sync_failed";
  if (failed(run?.transform_status)) return "transform_failed";

  if (inFlight(run?.status) || inFlight(sync?.status)) return "in_flight";

  // History outlives configuration by design, so a connector with runs and no
  // configuration is not "never configured" — it is one nothing manages now.
  if (!row.configured) return "unmanaged";

  // A sync the pipeline did not perform runs no transform of its own, so the
  // downstream layers were not rebuilt. True whenever that sync is the newest
  // thing to have happened, not only when no run exists at all.
  if (sync?.trigger === "out_of_band" && (!run || isNewer(sync.started_at, run.started_at))) {
    return "sync_without_transform";
  }

  if (!run && !sync) return "never_ran";

  // After the known-bad readings, before any reassurance.
  if (!recognised(run?.status) || !recognised(sync?.status)) return "state_unknown";

  // A connector whose runs succeed while nothing is ever stored is one of the
  // four states this page exists to separate, and it is not delivering.
  if (storage && storage.physical_rows === 0) return "nothing_stored";

  return "delivering";
}

export function connectorStateLabel(
  row: ConnectorHealthRow
): ConnectorStateLabel {
  const state = connectorState(row);
  const { label, tone } = STATES[state];
  return { state, label, tone };
}

const TRIGGER_LABELS: Record<SyncTrigger, string> = {
  claimed: "started by the pipeline",
  out_of_band: "started outside the pipeline",
  unclaimed: "origin unknown",
};

/**
 * How a sync's provenance reads.
 *
 * `unclaimed` says unknown, never "manual": the run that started it may simply
 * have aged out of the workflow layer's records. An unrecognised word reads the
 * same way rather than falling through to no label at all, which would be
 * indistinguishable from nothing recorded.
 */
export function triggerLabel(trigger: SyncTrigger | null): string | null {
  if (!trigger) return null;
  return TRIGGER_LABELS[trigger] ?? TRIGGER_LABELS.unclaimed;
}

/**
 * Elapsed wall-clock. A measured zero is a zero — only an absent measurement
 * reads as unknown, which is why the parameter is nullable.
 */
export function formatDuration(ms: number | null): string {
  if (ms === null) return "not recorded";
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** How long ago, from a recorded stamp. */
export function formatAge(iso: string | null, now: Date = new Date()): string {
  if (!iso) return "unknown";
  const elapsed = now.getTime() - new Date(iso).getTime();
  if (Number.isNaN(elapsed)) return "unknown";
  const minutes = Math.round(elapsed / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

/** A size, where zero is a measurement and only null is unknown. */
export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "unknown";
  if (bytes === 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

function count(value: number | null): string {
  return value === null ? "not recorded" : value.toLocaleString("en-US");
}

/**
 * The delivery pairing, as one readable cell.
 *
 * Each side says unknown on its own terms: the mover's counters arrive with the
 * sweep, the measurement only from the pipeline. Rendering either as zero would
 * invent the mismatch this pairing exists to find.
 */
export function formatDelivery(
  recordsMoved: number | null,
  rowsLanded: number | null
): string {
  const landed = rowsLanded === null ? "not measured" : rowsLanded.toLocaleString("en-US");
  return `${count(recordsMoved)} / ${landed}`;
}

/** Counts per state, for the tiles above the table. */
export function stateCounts(
  rows: ConnectorHealthRow[]
): Map<ConnectorState, number> {
  const counts = new Map<ConnectorState, number>();
  for (const row of rows) {
    const state = connectorState(row);
    counts.set(state, (counts.get(state) ?? 0) + 1);
  }
  return counts;
}
