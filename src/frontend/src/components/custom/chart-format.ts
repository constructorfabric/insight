/**
 * How a chart writes what it draws.
 *
 * Axis ticks come straight out of the warehouse: a `DateTime64` day arrives as
 * `2024-11-23 00:00:00.000`, a commit sha is 40 characters, and a sum of
 * seconds is `163997048`. Printed raw they crowd the axis and read as noise.
 */

const COMPACT = new Intl.NumberFormat("en", {
  notation: "compact",
  maximumFractionDigits: 1,
});
const GROUPED = new Intl.NumberFormat("en", { maximumFractionDigits: 1 });
/** Below ten, one decimal hides the difference between 2.5 and 2.54. */
const GROUPED_SMALL = new Intl.NumberFormat("en", { maximumFractionDigits: 2 });
const DAY = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const DAY_IN_YEAR = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
});

/** The longest a categorical tick may be before it is cut. */
const TICK_CHARS = 14;

/** `24.9k` for an axis, where the digits matter less than the shape. */
export function compactNumber(value: unknown, unit = ""): string {
  const number = toNumber(value);
  return number === null ? blank(value) : `${COMPACT.format(number)}${unit}`;
}

/** `163,997,048`, `37.7`, or `83.9%` — a figure someone reads, with its unit. */
export function groupedNumber(value: unknown, unit = ""): string {
  const number = toNumber(value);
  if (number === null) return blank(value);

  const format = Math.abs(number) < 10 ? GROUPED_SMALL : GROUPED;
  return `${format.format(number)}${unit}`;
}

/**
 * What a column's numbers are counted in.
 *
 * A rate arrives already multiplied by a hundred, so `83.9` and `83.9%` are
 * the same number until the metric says which — and only the metric knows.
 */
export function unitFor(
  percents: string[] | undefined,
  column: string
): string {
  return percents?.includes(column) ? "%" : "";
}

/**
 * A date as a person writes one, and anything else untouched.
 *
 * The warehouse hands back `2024-11-23`, `2024-11-23 00:00:00` or
 * `2024-11-23 00:00:00.000` depending on the column's type, and a year is
 * only worth the space when the data spans more than one.
 */
export function shortDate(value: unknown, withYear = false): string {
  const text = String(value ?? "");
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(text);
  if (!match) return text;

  const date = new Date(`${match[1]}-${match[2]}-${match[3]}T00:00:00Z`);
  if (Number.isNaN(date.getTime())) return text;

  return (withYear ? DAY_IN_YEAR : DAY).format(date);
}

/** A tick for a category: dates as dates, long strings cut with an ellipsis. */
export function categoryTick(value: unknown, withYear = false): string {
  const text = String(value ?? "");
  const asDate = shortDate(text, withYear);
  if (asDate !== text) return asDate;

  return text.length > TICK_CHARS ? `${text.slice(0, TICK_CHARS - 1)}…` : text;
}

/** Whether these values span more than one calendar year. */
export function spansYears(values: unknown[]): boolean {
  const years = new Set(
    values
      .map((value) => /^(\d{4})-/.exec(String(value ?? ""))?.[1])
      .filter(Boolean)
  );

  return years.size > 1;
}

function toNumber(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim() !== "") {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

function blank(value: unknown): string {
  return value === null || value === undefined ? "—" : String(value);
}
