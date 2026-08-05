/**
 * Local-timezone aware: a "week" means 7 local days. ISO output is
 * YYYY-MM-DD without a timezone suffix — the backend filters by date string.
 */

import { differenceInCalendarDays, isValid, parseISO } from "date-fns";

import type { CustomRange, PeriodValue } from "@/types/insight";

export type DateRange = { from: string; to: string };

const ISO_DATE_RE = /^\d{4}-\d{2}-\d{2}$/;

export const MAX_DATE_RANGE_DAYS = 400;

export type DateRangeValidation =
  | { valid: true; days: number }
  | {
      valid: false;
      reason: "invalid_date" | "inverted" | "too_long";
      days?: number;
    };

function parseISOCalendarDate(value: string): Date | null {
  if (!ISO_DATE_RE.test(value)) return null;

  const [year, month, day] = value.split("-").map(Number);
  const date = parseISO(value);

  if (
    !isValid(date) ||
    date.getFullYear() !== year ||
    date.getMonth() !== month! - 1 ||
    date.getDate() !== day
  ) {
    return null;
  }

  return date;
}

export function validateDateRange(range: DateRange): DateRangeValidation {
  const from = parseISOCalendarDate(range.from);
  const to = parseISOCalendarDate(range.to);

  if (!from || !to) return { valid: false, reason: "invalid_date" };
  if (from > to) return { valid: false, reason: "inverted" };

  const days = differenceInCalendarDays(to, from) + 1;

  if (days > MAX_DATE_RANGE_DAYS) {
    return { valid: false, reason: "too_long", days };
  }

  return { valid: true, days };
}

export function toISODate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function localToday(): Date {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return today;
}

function subtractMonths(d: Date, months: number): Date {
  const out = new Date(d);
  const targetMonth = out.getMonth() - months;
  const originalDay = out.getDate();
  out.setDate(1);
  out.setMonth(targetMonth);
  const lastDay = new Date(out.getFullYear(), out.getMonth() + 1, 0).getDate();
  out.setDate(Math.min(originalDay, lastDay));
  return out;
}

export function periodToDateRange(period: PeriodValue): DateRange {
  const today = localToday();
  const to = new Date(today);
  to.setDate(to.getDate() - 1);
  let from: Date;

  switch (period) {
    case "week":
      from = new Date(to);
      from.setDate(from.getDate() - 6);
      break;
    case "month":
      from = subtractMonths(to, 1);
      from.setDate(from.getDate() + 1);
      break;
    case "quarter":
      from = subtractMonths(to, 3);
      from.setDate(from.getDate() + 1);
      break;
    case "year": {
      from = new Date(to);
      from.setFullYear(from.getFullYear() - 1);
      from.setDate(from.getDate() + 1);
      break;
    }
  }

  return { from: toISODate(from), to: toISODate(to) };
}

function assertDateRange(range: DateRange): void {
  const validation = validateDateRange(range);
  if (validation.valid) return;

  if (validation.reason === "too_long") {
    throw new Error(
      `Date range exceeds ${MAX_DATE_RANGE_DAYS} days: from=${range.from} to=${range.to}`
    );
  }

  throw new Error(`Invalid date range: from=${range.from} to=${range.to}`);
}

export function resolveDateRange(
  period: PeriodValue,
  customRange: CustomRange | null
): DateRange {
  if (customRange) {
    const range: DateRange = { from: customRange.from, to: customRange.to };
    assertDateRange(range);
    return range;
  }
  return periodToDateRange(period);
}

// Month/quarter shifts clamp to the last day of the target month so
// end-of-month boundaries don't roll forward.
export function previousPeriodRange(
  range: DateRange,
  period: PeriodValue
): DateRange {
  const shift = (iso: string): string => {
    const [y, m, d] = iso.split("-").map(Number);
    const date = new Date(y, (m ?? 1) - 1, d ?? 1);
    const originalDay = date.getDate();

    const shiftMonths = (months: number): void => {
      date.setDate(1);
      date.setMonth(date.getMonth() - months);
      const lastDay = new Date(
        date.getFullYear(),
        date.getMonth() + 1,
        0
      ).getDate();
      date.setDate(Math.min(originalDay, lastDay));
    };

    switch (period) {
      case "week":
        date.setDate(date.getDate() - 7);
        break;
      case "month":
        shiftMonths(1);
        break;
      case "quarter":
        shiftMonths(3);
        break;
      case "year":
        date.setFullYear(date.getFullYear() - 1);
        break;
    }
    return toISODate(date);
  };
  return { from: shift(range.from), to: shift(range.to) };
}

