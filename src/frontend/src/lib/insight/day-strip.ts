import type { DayReading } from "@/lib/insight/metric-grain";

export interface StripDay {
  date: string;
  /** Null where the day has no reading at all — never a stand-in zero. */
  value: number | null;
  /** False where the source has not delivered this day yet. */
  collected: boolean;
  /** True where the day is delivered but its suppliers may still revise it. */
  provisional: boolean;
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
 *
 * `collectedThrough` splits that silence again: past it the source has not
 * delivered the day yet, so nothing can be said about it — before it, silence
 * is the answer. Null leaves every day collected, which is what the catalogue
 * reports for a metric it cannot date rather than one nobody has collected.
 *
 * `revisionWindowDays` marks the delivered days the suppliers may still change.
 * Their readings are real and are drawn as such — the mark says the figure is
 * not yet final, not that it is absent.
 */
export function stripDays(
  readings: DayReading[],
  from: string,
  to: string,
  collectedThrough?: string | null,
  revisionWindowDays?: number | null
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
  // The first date that has settled: everything from here to the boundary is
  // still open to revision. Absent window means everything settles on arrival.
  const settledBefore =
    collectedThrough != null && revisionWindowDays != null
      ? new Date(Date.parse(`${collectedThrough}T00:00:00Z`) - revisionWindowDays * DAY_MS)
          .toISOString()
          .slice(0, 10)
      : null;
  return days.map((date) => {
    const collected = collectedThrough == null || date <= collectedThrough;
    const provisional = collected && settledBefore != null && date > settledBefore;
    const reading = byDate.get(date);
    if (!reading) {
      return {
        date,
        collected,
        provisional,
        value: null,
        height: null,
        numerator: null,
        denominator: null,
      };
    }
    return {
      date,
      collected,
      provisional,
      value: reading.value,
      height: peak > 0 ? reading.value / peak : 0,
      numerator: reading.numerator,
      denominator: reading.denominator,
    };
  });
}

/** How many collected days of the period carry no reading. */
export function silentDays(days: StripDay[]): number {
  return days.filter((d) => d.collected && d.value == null).length;
}

/** How many days the source has not delivered yet. */
export function uncollectedDays(days: StripDay[]): number {
  return days.filter((d) => !d.collected).length;
}

/** How many delivered days may still be revised. */
export function provisionalDays(days: StripDay[]): number {
  return days.filter((d) => d.provisional).length;
}
