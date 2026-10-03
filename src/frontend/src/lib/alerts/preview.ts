/**
 * What a check would read from a metric's answer right now.
 *
 * The same rule the service applies to a check: exactly one row, the named
 * column, a finite JSON number. A numeric string is not a number to the check
 * either. This only previews; the service's own check is what counts.
 */

import type { AlertOperator, UnknownReason } from "@/api/alerts-types";
import type { MetricResult } from "@/api/custom-types";

export type Preview =
  | { kind: "value"; value: number; breached: boolean }
  | { kind: "unknown"; reason: UnknownReason };

function meets(value: number, operator: AlertOperator, threshold: number) {
  switch (operator) {
    case ">":
      return value > threshold;
    case ">=":
      return value >= threshold;
    case "<":
      return value < threshold;
    case "<=":
      return value <= threshold;
  }
}

/**
 * Whether an integer and a fractional number can be compared exactly. The
 * service refuses where the integer is past what a float holds exactly.
 */
function comparable(value: number, threshold: number): boolean {
  const mixed = Number.isInteger(value) !== Number.isInteger(threshold);
  if (!mixed) return true;

  const integer = Number.isInteger(value) ? value : threshold;
  return Number.isSafeInteger(integer);
}

export function previewCheck(
  result: MetricResult,
  column: string,
  operator: AlertOperator,
  threshold: number
): Preview {
  const at = result.columns.indexOf(column);
  if (at < 0) return { kind: "unknown", reason: "column_missing" };
  if (result.rows.length === 0) return { kind: "unknown", reason: "no_rows" };
  if (result.rows.length > 1) return { kind: "unknown", reason: "many_rows" };

  const value = result.rows[0][at];
  if (value === null || value === undefined) {
    return { kind: "unknown", reason: "null" };
  }
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return { kind: "unknown", reason: "non_numeric" };
  }
  if (!comparable(value, threshold)) {
    return { kind: "unknown", reason: "incomparable" };
  }

  return { kind: "value", value, breached: meets(value, operator, threshold) };
}
