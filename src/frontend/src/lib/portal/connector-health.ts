import type { ConnectorRow } from "@/api/connector-health-client";

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

/**
 * Oldest data first, and connectors that never delivered last: a source that
 * stopped needs someone today, while one that was never wired here is the
 * expected state on an instance that does not use it.
 */
export function orderByAttention(rows: ConnectorRow[]): ConnectorRow[] {
  return [...rows].sort((a, b) => {
    if (a.last_write == null || b.last_write == null) {
      return Number(a.last_write == null) - Number(b.last_write == null);
    }
    return Date.parse(a.last_write) - Date.parse(b.last_write);
  });
}

/**
 * How long since data arrived, as a shape the view translates. Kept free of
 * copy so pluralisation stays with i18next rather than being hand-rolled here.
 */
export type Elapsed =
  | { kind: "never" }
  | { kind: "hours"; value: number }
  | { kind: "days"; value: number };

export function elapsedSince(lastWrite: string | null, now: Date): Elapsed {
  if (lastWrite == null) return { kind: "never" };

  const elapsed = now.getTime() - Date.parse(lastWrite);
  if (elapsed < DAY_MS) {
    return { kind: "hours", value: Math.floor(elapsed / HOUR_MS) };
  }
  return { kind: "days", value: Math.floor(elapsed / DAY_MS) };
}
