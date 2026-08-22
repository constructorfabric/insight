import type { ConnectorRow } from "@/api/connector-health-client";

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

export function orderByAttention(rows: ConnectorRow[]): ConnectorRow[] {
  return [...rows].sort((a, b) => {
    if (a.last_write == null || b.last_write == null) {
      return Number(a.last_write == null) - Number(b.last_write == null);
    }
    return Date.parse(a.last_write) - Date.parse(b.last_write);
  });
}

export type Elapsed =
  | { kind: "never" }
  | { kind: "unknown" }
  | { kind: "hours"; value: number }
  | { kind: "days"; value: number };

export function elapsedSince(lastWrite: string | null, now: Date): Elapsed {
  if (lastWrite == null) return { kind: "never" };

  const written = Date.parse(lastWrite);
  if (Number.isNaN(written)) return { kind: "unknown" };

  // INVARIANT: the arrival is stamped by another host's clock than the read,
  // so skew can put it in the future.
  const elapsed = Math.max(0, now.getTime() - written);
  if (elapsed < DAY_MS) {
    return { kind: "hours", value: Math.floor(elapsed / HOUR_MS) };
  }
  return { kind: "days", value: Math.floor(elapsed / DAY_MS) };
}

export type ConnectorState = "delivering" | "partial" | "never";

export function connectorState(row: ConnectorRow): ConnectorState {
  if (row.last_write == null) return "never";
  return row.streams_with_data < row.streams ? "partial" : "delivering";
}
