import type { DayReading } from "@/lib/insight/metric-grain";

export interface StripDay {
  date: string;
  /** Null where the day has no reading at all — never a stand-in zero. */
  value: number | null;
  /** Share of the tallest reading, 0..1; null follows `value`. */
  height: number | null;
  numerator: number | null;
  denominator: number | null;
}

const DAY_MS = 86_400_000;

/** Every date from `from` to `to`, inclusive. */
function calendar(from: string, to: string): string[] {
  const start = Date.parse(`${from}T00:00:00Z`);
  const end = Date.parse(`${to}T00:00:00Z`);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start)
    return [];
  const days: string[] = [];
  for (let t = start; t <= end; t += DAY_MS) {
    days.push(new Date(t).toISOString().slice(0, 10));
  }
  return days;
}

/**
 * The period's days in order, each carrying its reading or nothing.
 *
 * Built from the calendar rather than from the rows, because the rows only
 * cover days something happened on. A strip drawn from rows alone packs the
 * active days together and hides every quiet one — the shape of a month with
 * four busy days would be indistinguishable from a month that was busy
 * throughout.
 *
 * A day with no reading stays null and a measured zero stays zero. They look
 * the same on a bar chart and mean opposite things: one is silence from the
 * source, the other is a day this person did none of it.
 */
export function stripDays(
  readings: DayReading[],
  from: string,
  to: string
): StripDay[] {
  const byDate = new Map(readings.map((r) => [r.date, r]));
  const days = calendar(from, to);
  // Scaled to the tallest day ON THE STRIP, not the tallest reading handed in.
  // A reading outside the window would otherwise set the height of bars it is
  // not drawn beside, and every one of them would read as smaller than it is.
  const peak = days.reduce(
    (max, date) => Math.max(max, byDate.get(date)?.value ?? 0),
    0
  );
  return days.map((date) => {
    const reading = byDate.get(date);
    if (!reading) {
      return {
        date,
        value: null,
        height: null,
        numerator: null,
        denominator: null,
      };
    }
    return {
      date,
      value: reading.value,
      height: peak > 0 ? reading.value / peak : 0,
      numerator: reading.numerator,
      denominator: reading.denominator,
    };
  });
}

/** How many days of the period carry no reading. */
export function silentDays(days: StripDay[]): number {
  return days.filter((d) => d.value == null).length;
}
