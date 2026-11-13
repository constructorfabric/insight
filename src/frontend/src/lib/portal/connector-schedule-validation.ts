interface Violation {
  field: string;
  description: string;
}

interface CronField {
  values: Set<number>;
  star: boolean;
}

const BOUNDS = [
  [0, 59],
  [0, 23],
  [1, 31],
  [1, 12],
  [0, 6],
];
const FIELDS = ["minute", "hour", "day of month", "month", "day of week"];
const MONTHS = [
  "JAN",
  "FEB",
  "MAR",
  "APR",
  "MAY",
  "JUN",
  "JUL",
  "AUG",
  "SEP",
  "OCT",
  "NOV",
  "DEC",
];
const DAYS = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

function decimal(raw: string): number | null {
  if (!/^[0-9]+$/.test(raw)) return null;
  const number = Number(raw);
  return number <= 4_294_967_295 ? number : null;
}

function value(raw: string, index: number): number {
  const names = index === 3 ? MONTHS : index === 4 ? DAYS : [];
  const named = /^[a-z]+$/i.test(raw) ? names.indexOf(raw.toUpperCase()) : -1;
  const number =
    decimal(raw) ?? (named < 0 ? null : named + (index === 3 ? 1 : 0));
  if (number === null)
    throw new Error(
      `Unsupported value ${raw}; L, W, # and misplaced ? are not supported.`
    );
  const [min, max] = BOUNDS[index];
  if (number < min || number > max)
    throw new Error(`Value ${raw} is outside ${min}–${max}.`);
  return number;
}

function field(raw: string, index: number): CronField {
  if (raw === "?" && (index === 2 || index === 4)) return field("*", index);
  const [min, max] = BOUNDS[index];
  const result: CronField = { values: new Set(), star: false };
  for (const item of raw.split(",")) {
    if (!item) throw new Error("Empty list item.");
    const slash = item.indexOf("/");
    const base = slash < 0 ? item : item.slice(0, slash);
    const step = slash < 0 ? 1 : decimal(item.slice(slash + 1));
    if (step === null || step === 0)
      throw new Error("Step must be a positive integer.");
    let start: number;
    let end: number;
    if (base === "*") {
      result.star ||= step === 1;
      [start, end] = [min, max];
    } else if (base.includes("-")) {
      const dash = base.indexOf("-");
      start = value(base.slice(0, dash), index);
      end = value(base.slice(dash + 1), index);
    } else {
      start = value(base, index);
      end = slash < 0 ? start : max;
    }
    if (start > end) throw new Error("Range start exceeds range end.");
    for (let number = start; number <= end; number += step)
      result.values.add(number);
  }
  return result;
}

function expression(raw: string): string {
  if (Array.from(raw).length > 100)
    throw new Error("Cron expression exceeds 100 characters.");
  if (
    Array.from(raw).some((char) => {
      const code = char.codePointAt(0)!;
      return (code < 32 && code !== 9) || (code >= 127 && code <= 159);
    })
  )
    throw new Error(
      "Cron expression contains control characters or line breaks."
    );
  const normalized = raw
    .split(/[ \t]+/)
    .filter(Boolean)
    .join(" ");
  if (normalized.includes("@"))
    throw new Error("@ descriptors are not supported.");
  if (normalized.includes("TZ="))
    throw new Error("TZ= and CRON_TZ= prefixes are not supported.");
  const parts = normalized.split(" ");
  if (parts.length !== 5)
    throw new Error(
      "Cron expression must have exactly five fields (no seconds or years)."
    );
  const fields = parts.map((part, index) => {
    try {
      return field(part, index);
    } catch (error) {
      throw new Error(`${FIELDS[index]}: ${(error as Error).message}`, {
        cause: error,
      });
    }
  });
  if (fields[2].star || fields[4].star) {
    const days = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    const maxDay = Math.max(
      ...[...fields[3].values].map((month) => days[month - 1])
    );
    if (![...fields[2].values].some((day) => day <= maxDay))
      throw new Error(
        "Expression never runs: no listed month has any listed day of month (including leap years)."
      );
  }
  return normalized;
}

export function validateScheduleCrons(crons: string[]): Violation[] {
  if (crons.length < 1 || crons.length > 10)
    return [
      { field: "crons", description: "Expected 1 to 10 cron expressions." },
    ];
  const normalized: string[] = [];
  const errors: Violation[] = [];
  crons.forEach((cron, index) => {
    try {
      const parsed = expression(cron);
      const previous = normalized.indexOf(parsed);
      normalized[index] = parsed;
      if (previous >= 0)
        throw new Error(
          `Duplicates crons[${previous}] after whitespace normalization.`
        );
    } catch (error) {
      errors.push({
        field: `crons[${index}]`,
        description: (error as Error).message,
      });
    }
  });
  return errors;
}
