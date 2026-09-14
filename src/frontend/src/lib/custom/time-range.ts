/**
 * The windows a custom dashboard can be read over.
 *
 * The token is what crosses the wire and sits in the URL; the label is only
 * ever shown. The server resolves every token against the clock, so nothing
 * here computes a date — a custom interval is passed on as the two dates the
 * reader picked.
 */

import { parseISOCalendarDate } from "@/api/period-to-date-range";

export interface RangePreset {
  token: string;
  label: string;
}

export const RANGE_PRESETS: readonly RangePreset[] = [
  { token: "PDC", label: "Yesterday" },
  { token: "P7D", label: "Last 7 days" },
  { token: "P30D", label: "Last 30 days" },
  { token: "PMC", label: "Last month" },
  { token: "PQC", label: "Last quarter" },
  { token: "P1Y", label: "Last 365 days" },
  { token: "inf", label: "All time" },
] as const;

const LABELS = new Map(RANGE_PRESETS.map(({ token, label }) => [token, label]));

export interface DateInterval {
  from: string;
  to: string;
}

/** The two dates a custom range names, or nothing when it names a preset. */
export function toInterval(token: string): DateInterval | undefined {
  const parts = token.split("/");
  if (parts.length !== 2) return undefined;

  const [from, to] = parts;
  if (!parseISOCalendarDate(from) || !parseISOCalendarDate(to)) return undefined;
  if (from >= to) return undefined;

  return { from, to };
}

/**
 * Whether the server would accept this as a range.
 *
 * The same check the handler makes, run here so a hand-edited URL degrades to
 * the board's default instead of a 400.
 */
export function isRangeToken(token: string): boolean {
  return LABELS.has(token) || toInterval(token) !== undefined;
}

export function rangeLabel(token: string): string {
  const preset = LABELS.get(token);
  if (preset) return preset;

  const interval = toInterval(token);
  return interval ? `${interval.from} – ${interval.to}` : token;
}

export function toRangeToken({ from, to }: DateInterval): string {
  return `${from}/${to}`;
}
