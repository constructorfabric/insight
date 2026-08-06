import { format, parseISO } from "date-fns";
import { enUS } from "date-fns/locale";

import type { MetricFormat } from "@/api/metric-results-client";
import { LOCALE } from "@/config/constants";

const NF_THOUSANDS = new Intl.NumberFormat(LOCALE);

const METRIC_CURRENCY_FORMAT = new Intl.NumberFormat(LOCALE, {
  style: "currency",
  currency: "USD",
  maximumFractionDigits: 0,
});

/**
 * The em-dash stands in for a metric that has no honest value — a null,
 * undefined, or non-finite result. It reads as "no data", never as a real
 * zero (insight#1977).
 */
export const NO_METRIC_VALUE = "—";

/**
 * Formatting for `/v1/metric-results` values: the wire `format` decides
 * rounding and presentation; `unit` is a display suffix only. Bare number —
 * no unit/percent suffix (use `formatMetricValue` for the suffixed form,
 * `metricDisplayUnit` when the unit renders as a separate element).
 *
 * A null/undefined/non-finite value renders as `NO_METRIC_VALUE`, never a
 * fabricated `0`: honesty is guaranteed here rather than left to each caller.
 */
export function formatMetricNumber(
  v: number | null | undefined,
  fmt: MetricFormat,
): string {
  if (v == null || !Number.isFinite(v)) return NO_METRIC_VALUE;
  if (fmt === "currency") return METRIC_CURRENCY_FORMAT.format(v);
  const rounded = fmt === "decimal" ? Math.round(v * 10) / 10 : Math.round(v);
  return NF_THOUSANDS.format(rounded);
}

export function formatMetricValue(
  v: number | null | undefined,
  fmt: MetricFormat,
  unit?: string | null,
): string {
  if (v == null || !Number.isFinite(v)) return NO_METRIC_VALUE;
  const s = formatMetricNumber(v, fmt);
  if (fmt === "currency") return s;
  if (fmt === "percent") return `${s}%`;
  return unit ? `${s} ${unit}` : s;
}

/** Unit rendered beside the number; none when the number carries it. */
export function metricDisplayUnit(
  fmt: MetricFormat,
  unit?: string | null,
): string | undefined {
  if (fmt === "currency" || fmt === "percent") return undefined;
  return unit ?? undefined;
}

/**
 * Ratio at/above which a runaway signed percent ("+300%", "+5460%") renders
 * as a multiple instead — shared by the vs-median gap and the
 * period-over-period delta so both surfaces switch at the same point.
 */
export const MULTIPLE_THRESHOLD = 2;

/** "5.6×" under 10, "56×" above — a multiple needs no decimal at that size. */
export function formatMultiple(ratio: number): string {
  const rounded = ratio >= 10 ? Math.round(ratio) : Math.round(ratio * 10) / 10;
  return `${rounded}×`;
}

export function formatPp(diff: number, decimals = 1): string {
  const sign = diff > 0 ? "+" : diff < 0 ? "-" : "";
  return `${sign}${Math.abs(diff).toFixed(decimals)} pp`;
}

export function formatDate(iso: string, pattern = "d MMM"): string {
  return format(parseISO(iso), pattern, { locale: enUS });
}

/** "1.0k" reads worse than "1k" on an axis. */
function trimTrailingZero(s: string): string {
  return s.endsWith(".0") ? s.slice(0, -2) : s;
}

/**
 * Axis tick labels. Kept short on purpose: an axis gutter is ~28px, while
 * "36000" at the 10px tick font needs 31px and gets clipped by the chart's own
 * edge. Abbreviating makes the width independent of the magnitude, so a series
 * that grows an order of magnitude does not silently start truncating.
 *
 * The 10k threshold is where abbreviation stops costing accuracy: a tick sits on
 * a gridline, so "2.3k" for a line drawn at 2250 is a label that lies. Four
 * digits still fit; five do not.
 */
export function formatAxisTick(v: number): string {
  const abs = Math.abs(v);
  if (abs >= 1_000_000) return `${trimTrailingZero((v / 1_000_000).toFixed(1))}M`;
  if (abs >= 10_000) {
    const k = Math.round(v / 1000);
    // 999_600 rounds to 1000k — a tick that names a magnitude the scale below
    // it already covers. Roll it over rather than print a fourth digit.
    if (Math.abs(k) >= 1000) return `${trimTrailingZero((k / 1000).toFixed(1))}M`;
    return `${k}k`;
  }
  return trimTrailingZero((Math.round(v * 10) / 10).toFixed(1));
}
