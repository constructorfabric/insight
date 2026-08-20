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
