/**
 * Which windows make sense for the column an alert watches.
 *
 * A metric with no date answers every row and refuses a window. A metric with
 * a date can be read over every row, but a total or a count read that way only
 * ever grows, so the threshold it crosses once it crosses forever.
 */

import type { StoredMetric } from "@/api/custom-types";

/** The window a dated metric starts on, when its alert has none yet. */
export const DEFAULT_WINDOW = "P7D";

const GROWING = new Set(["sum", "count"]);

export interface WindowRule {
  /** Every row may be chosen. */
  everyRow: boolean;
  /** A window may be chosen. */
  windowed: boolean;
  /** Why a choice is closed, said beside the control. */
  why?: string;
}

export function windowRule(
  stored: StoredMetric | undefined,
  column: string
): WindowRule {
  if (!stored) return { everyRow: true, windowed: true };
  if (!stored.clock) {
    return {
      everyRow: true,
      windowed: false,
      why: "This metric has no date, so it is always read over every row.",
    };
  }

  const field = stored.definition.fields.find((one) => one.as_name === column);
  if (field?.agg && GROWING.has(field.agg)) {
    return {
      everyRow: false,
      windowed: true,
      why: "A total over every row only grows, so it needs a window.",
    };
  }

  return { everyRow: true, windowed: true };
}

/**
 * The window a choice of column moves the rule to: a window where the column
 * needs one, none where the metric cannot take one, the default where nothing
 * was picked yet, and otherwise what was already chosen.
 */
export function windowFor(
  stored: StoredMetric | undefined,
  column: string,
  current: string
): string {
  const rule = windowRule(stored, column);
  if (!rule.windowed) return "";
  if (current === "" && stored?.clock) return DEFAULT_WINDOW;

  return current;
}
