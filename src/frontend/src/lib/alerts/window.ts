/**
 * Whether the metric an alert watches can be read over a window.
 *
 * A metric with no date answers all of its rows and refuses a window. A
 * metric with a date takes a window, or is read over all time.
 */

import type { StoredMetric } from "@/api/custom-types";

/** The window a dated metric starts on, when its alert has none yet. */
const DEFAULT_WINDOW = "P7D";

/** A metric not read yet is taken to have a date until it says otherwise. */
export function takesWindow(stored: StoredMetric | undefined): boolean {
  return stored === undefined || Boolean(stored.clock);
}

/**
 * The window a choice of column moves the rule to: none where the metric
 * cannot take one, the default where nothing was picked yet, and otherwise
 * what was already chosen.
 */
export function windowFor(
  stored: StoredMetric | undefined,
  current: string
): string {
  if (stored === undefined) return current;
  if (!stored.clock) return "";

  return current === "" ? DEFAULT_WINDOW : current;
}
