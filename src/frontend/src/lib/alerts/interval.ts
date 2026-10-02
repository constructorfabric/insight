/** How often an alert is checked, in the units a reader picks it in. */

export type IntervalUnit = "minutes" | "hours" | "days";

const UNIT_SECS: Record<IntervalUnit, number> = {
  minutes: 60,
  hours: 3_600,
  days: 86_400,
};

/** Largest unit first, so a whole number of days is read as days. */
const UNITS_LARGEST_FIRST: readonly IntervalUnit[] = [
  "days",
  "hours",
  "minutes",
];

/** The intervals offered as one click, smallest first. */
export const INTERVAL_PRESETS_SECS: readonly number[] = [
  60, 300, 900, 3_600, 21_600, 86_400, 604_800,
];

export interface IntervalInput {
  amount: number;
  unit: IntervalUnit;
}

export function toSeconds({ amount, unit }: IntervalInput): number {
  return amount * UNIT_SECS[unit];
}

/**
 * Seconds as the largest unit that holds them exactly.
 *
 * An interval the service stores but no unit divides (90 seconds, say) is
 * shown in minutes rounded up, so it is never shown as shorter than it is.
 */
export function fromSeconds(secs: number): IntervalInput {
  const exact = UNITS_LARGEST_FIRST.find(
    (unit) => secs % UNIT_SECS[unit] === 0
  );
  if (exact) return { amount: secs / UNIT_SECS[exact], unit: exact };

  return { amount: Math.ceil(secs / UNIT_SECS.minutes), unit: "minutes" };
}

const SINGULAR: Record<IntervalUnit, string> = {
  minutes: "minute",
  hours: "hour",
  days: "day",
};

/** "Every 5 minutes", "Every hour", "Every 7 days". */
export function intervalText(secs: number): string {
  if (secs < UNIT_SECS.minutes) return `Every ${secs} seconds`;

  const { amount, unit } = fromSeconds(secs);
  if (amount === 1) return `Every ${SINGULAR[unit]}`;

  return `Every ${amount} ${unit}`;
}
