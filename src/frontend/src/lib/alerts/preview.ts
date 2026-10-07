/**
 * What a check would read from a metric's answer right now.
 *
 * The same rule the service applies to a check: exactly one row, the named
 * column, a finite JSON number, compared exactly — a whole number with a
 * whole number, a fraction with a fraction, and across the two only where the
 * whole number is one a float holds exactly. A numeric string is not a number
 * to the check either. This only previews; the service's own check is what
 * counts.
 */

import type {
  AlertNumber,
  AlertOperator,
  UnknownReason,
} from "@/api/alerts-types";
import type { MetricResult } from "@/api/custom-types";

export type Preview =
  | { kind: "value"; value: number; breached: boolean | undefined }
  /** A whole number past what JSON parsing holds exactly: shown about, never judged. */
  | { kind: "wide"; value: number }
  | { kind: "unknown"; reason: UnknownReason };

/** A number as the check holds it: whole numbers exactly, fractions as floats. */
type Exact = { whole: bigint } | { fraction: number };

const WHOLE = /^-?\d+$/;
/** The widest whole number a float still holds exactly: the service's own bound, 2^53. */
const EXACT_FLOAT_BOUND = 2n ** 53n;

function exactOf(number: AlertNumber): Exact | undefined {
  if (typeof number === "string") {
    return WHOLE.test(number) ? { whole: BigInt(number) } : undefined;
  }
  if (!Number.isFinite(number)) return undefined;
  if (Number.isInteger(number)) {
    // INVARIANT: a whole number past 2^53 was rounded on the way in, so its
    // digits are not the service's; only a digit string carries those.
    return Number.isSafeInteger(number) ? { whole: BigInt(number) } : undefined;
  }

  return { fraction: number };
}

/** Both sides as one kind, or nothing where a whole number is past what a float holds. */
function aligned(
  value: Exact,
  threshold: Exact
): [bigint, bigint] | [number, number] | undefined {
  if ("whole" in value && "whole" in threshold) {
    return [value.whole, threshold.whole];
  }
  if ("fraction" in value && "fraction" in threshold) {
    return [value.fraction, threshold.fraction];
  }

  const whole = "whole" in value ? value.whole : (threshold as { whole: bigint }).whole;
  if (whole > EXACT_FLOAT_BOUND || whole < -EXACT_FLOAT_BOUND) return undefined;
  const asFloat = (side: Exact) =>
    "whole" in side ? Number(side.whole) : side.fraction;

  return [asFloat(value), asFloat(threshold)];
}

function meets(
  operator: AlertOperator,
  [value, threshold]: [bigint, bigint] | [number, number]
): boolean {
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

export function previewCheck(
  result: MetricResult,
  column: string,
  operator: AlertOperator,
  threshold: AlertNumber | undefined
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
  if (Number.isInteger(value) && !Number.isSafeInteger(value)) {
    return { kind: "wide", value };
  }
  if (threshold === undefined) return { kind: "value", value, breached: undefined };

  const read = exactOf(value);
  const against = exactOf(threshold);
  const sides = read && against ? aligned(read, against) : undefined;
  if (!sides) return { kind: "unknown", reason: "incomparable" };

  return { kind: "value", value, breached: meets(operator, sides) };
}
