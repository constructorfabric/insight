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
 */
export type ConnectorState =
  /** Reported moving records while storage gained none. */
  | "misdelivered"
  /** The last run failed. */
  | "run_failed"
  /** The sync succeeded and this run's transform did not. */
  | "transform_failed"
  /** A sync ran and no transform followed it. */
  | "sync_without_transform"
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

const LABELS: Record<ConnectorState, Omit<ConnectorStateLabel, "state">> = {
  misdelivered: { label: "recorded, nothing landed", tone: "critical" },
  run_failed: { label: "run failed", tone: "critical" },
  transform_failed: { label: "transform failed", tone: "warning" },
  sync_without_transform: { label: "sync without transform", tone: "warning" },
  delivering: { label: "delivering", tone: "ok" },
  never_ran: { label: "never ran", tone: "idle" },
  not_configured: { label: "not configured", tone: "idle" },
};

function failed(status: string | null | undefined): boolean {
  return status === "failed" || status === "cancelled";
}

/**
 * The one place a state is decided.
 *
 * Order matters and is deliberate: a delivery mismatch outranks a failed run,
 * because a run that reports success while nothing landed is the more
 * misleading of the two.
 */
export function connectorState(row: ConnectorHealthRow): ConnectorState {
  if (!row.configured && !row.last_run && !row.last_sync) {
    return "not_configured";
  }

  const sync = row.last_sync;
  // A null measurement is absence: only a measured zero is a mismatch.
  if (sync && sync.records_moved > 0 && sync.rows_landed === 0) {
    return "misdelivered";
  }

  const run = row.last_run;
  if (run) {
    if (failed(run.status)) return "run_failed";
    if (failed(run.transform_status)) return "transform_failed";
  }

  // A sync the pipeline did not perform runs no transform of its own, so the
  // downstream layers were not rebuilt. Worth saying, and only sayable because
  // the trigger was recorded.
  if (sync && !run && sync.trigger === "out_of_band") {
    return "sync_without_transform";
  }

  if (!run && !sync) return "never_ran";
  return "delivering";
}

export function connectorStateLabel(
  row: ConnectorHealthRow
): ConnectorStateLabel {
  const state = connectorState(row);
  return { state, ...LABELS[state] };
}

const TRIGGER_LABELS: Record<SyncTrigger, string> = {
  claimed: "scheduled",
  out_of_band: "started outside the pipeline",
  unclaimed: "origin unknown",
};

/**
 * How a sync's provenance reads.
 *
 * `unclaimed` says unknown, never "manual": the run that started it may simply
 * have aged out of the workflow layer's records.
 */
export function triggerLabel(trigger: SyncTrigger | null): string | null {
  return trigger ? TRIGGER_LABELS[trigger] : null;
}

/** Elapsed wall-clock, at the coarsest unit that still tells the operator something. */
export function formatDuration(ms: number): string {
  if (ms <= 0) return "—";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** How long ago, from a recorded stamp. */
export function formatAge(iso: string, now: Date = new Date()): string {
  const elapsed = now.getTime() - new Date(iso).getTime();
  if (Number.isNaN(elapsed)) return "—";
  const minutes = Math.round(elapsed / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/**
 * The delivery pairing, as one readable cell.
 *
 * Null is rendered as unknown rather than as a zero — the distinction is the
 * whole reason both numbers are recorded.
 */
export function formatDelivery(
  recordsMoved: number,
  rowsLanded: number | null
): string {
  const moved = recordsMoved.toLocaleString("en-US");
  if (rowsLanded === null) return `${moved} / not measured`;
  return `${moved} / ${rowsLanded.toLocaleString("en-US")}`;
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
