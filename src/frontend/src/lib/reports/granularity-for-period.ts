import { differenceInCalendarDays, parseISO } from "date-fns";

import type { DateRange } from "@/api/period-to-date-range";
import type { ReportGranularity } from "@/api/reports-client";
import i18n from "@/i18n";

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

const SPLIT_KEY: Record<ReportGranularity, string> = {
  day: "reports.split.day",
  week: "reports.split.week",
  month: "reports.split.month",
  quarter: "reports.split.quarter",
  year: "reports.split.year",
};

const PERIOD_KEY: Record<ReportGranularity, string> = {
  day: "reports.period_name.day",
  week: "reports.period_name.week",
  month: "reports.period_name.month",
  quarter: "reports.period_name.quarter",
  year: "reports.period_name.year",
};

function periodDays(range: DateRange): number {
  return differenceInCalendarDays(parseISO(range.to), parseISO(range.from)) + 1;
}

function granularityFitsPeriod(
  granularity: ReportGranularity,
  range: DateRange
): boolean {
  return periodDays(range) >= MIN_PERIOD_DAYS[granularity];
}

function coarsestGranularity(range: DateRange): ReportGranularity {
  return (
    COARSEST_FIRST.find((granularity) =>
      granularityFitsPeriod(granularity, range)
    ) ?? "day"
  );
}

export function clampGranularity(
  granularity: ReportGranularity,
  range: DateRange
): ReportGranularity {
  return granularityFitsPeriod(granularity, range)
    ? granularity
    : coarsestGranularity(range);
}

export function periodTooShortReason(
  granularity: ReportGranularity,
  range: DateRange
): string | null {
  if (granularityFitsPeriod(granularity, range)) return null;

  const finer = coarsestGranularity(range);
  return i18n.t("reports.too_short", {
    split: i18n.t(SPLIT_KEY[granularity]),
    period: i18n.t(PERIOD_KEY[granularity]),
    finer: i18n.t(SPLIT_KEY[finer]),
  });
}
