/**
 * Which windows make sense for the column an alert watches.
 *
 * A metric with no date answers all of its rows and refuses a window. A
 * metric with a date can be read over all time, but a total or a count read
 * that way only ever grows, so the threshold it crosses once it crosses forever.
 */

import type { StoredMetric } from "@/api/custom-types";

/** The window a dated metric starts on, when its alert has none yet. */
export const DEFAULT_WINDOW = "P7D";

const GROWING = new Set(["sum", "count"]);

export interface WindowRule {
  /** All time may be chosen. */
  allTime: boolean;
  /** A window may be chosen. */
  windowed: boolean;
  /** Why a choice is closed, said beside the control. */
  why?: string;
}

export function windowRule(
  stored: StoredMetric | undefined,
  column: string
): WindowRule {
  if (!stored) return { allTime: true, windowed: true };
  if (!stored.clock) {
    return {
      allTime: true,
      windowed: false,
      why: "This metric has no date, so each check reads all of it.",
    };
  }

  const field = stored.definition.fields.find((one) => one.as_name === column);
  if (field?.agg && GROWING.has(field.agg)) {
    return {
      allTime: false,
      windowed: true,
      why: "A total over all time only grows, so it needs a window.",
    };
  }

  return { allTime: true, windowed: true };
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
