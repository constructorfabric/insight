import { differenceInCalendarDays, parseISO } from "date-fns";

import type { DateRange } from "@/api/period-to-date-range";
import type { ReportGranularity } from "@/lib/reports/rollup";

// The shortest a rolling window of each grain can be, so every period preset
// admits its own grain: February makes a month 28 days and a quarter 89.
const MIN_PERIOD_DAYS: Record<ReportGranularity, number> = {
  day: 1,
  week: 7,
  month: 28,
  quarter: 89,
  year: 365,
};

const COARSEST_FIRST: ReadonlyArray<ReportGranularity> = [
  "year",
  "quarter",
  "month",
  "week",
  "day",
];

const SPLIT_NAME: Record<ReportGranularity, string> = {
  day: "daily",
  week: "weekly",
  month: "monthly",
  quarter: "quarterly",
  year: "yearly",
};

const PERIOD_NAME: Record<ReportGranularity, string> = {
  day: "a day",
  week: "a week",
  month: "a month",
  quarter: "a quarter",
  year: "a year",
};

function periodDays(range: DateRange): number {
  return differenceInCalendarDays(parseISO(range.to), parseISO(range.from)) + 1;
}

export function granularityFitsPeriod(
  granularity: ReportGranularity,
  range: DateRange,
): boolean {
  return periodDays(range) >= MIN_PERIOD_DAYS[granularity];
}

function coarsestGranularity(range: DateRange): ReportGranularity {
  return (
    COARSEST_FIRST.find((grain) => granularityFitsPeriod(grain, range)) ?? "day"
  );
}

export function clampGranularity(
  granularity: ReportGranularity,
  range: DateRange,
): ReportGranularity {
  return granularityFitsPeriod(granularity, range)
    ? granularity
    : coarsestGranularity(range);
}

export function periodTooShortReason(
  granularity: ReportGranularity,
  range: DateRange,
): string | null {
  if (granularityFitsPeriod(granularity, range)) return null;
  return `A ${SPLIT_NAME[granularity]} split needs a period of at least ${PERIOD_NAME[granularity]} — pick ${SPLIT_NAME[coarsestGranularity(range)]} or finer`;
}
