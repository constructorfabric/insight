import type { MetricFormat } from "@/api/metric-results-client";
import {
  MULTIPLE_THRESHOLD,
  formatMetricValue,
  formatMultiple,
  formatPp,
} from "@/lib/format";

/**
 * The single "scale of divergence" formatter shared by every surface that
 * shows how far a value sits from its peer median — the drilldown peer story,
 * the KPI tiles, and the needs-attention list. Only the surrounding wording
 * differs per surface (verbose in the drilldown, compact on the dashboard);
 * the magnitude itself is computed once, here.
 *
 * At/above `MULTIPLE_THRESHOLD`× the median a signed percent runs away
 * (300%, 500%…), so the far-above side reads as a multiple ("5.6×"); nearer
 * gaps and the whole below-median side (bounded to −100%) stay a signed
 * percent; a sub-unit gap with no usable median falls back to a signed
 * absolute delta.
 */
function formatGapPct(gap: number): string | null {
  const pct = Math.round(Math.abs(gap) * 100);
  if (pct === 0) return null;
  return `${gap >= 0 ? "+" : "-"}${pct}%`;
}

export interface GapInput {
  value: number;
  /** Peer-cohort median; null when suppressed. */
  median: number | null;
  /** Arithmetic (value − median) / |median|; null when median ~ 0. */
  gapPct: number | null;
  /** Arithmetic value − median. */
  gapDelta: number;
  format: MetricFormat;
  unit: string | null;
}

/**
 * Returns `null` when the magnitude would render as zero — a "0 pp" or "0%"
 * beside two visibly equal numbers reads as a bug, so callers drop the gap
 * text (or say "at the median") instead.
 */
export function formatGapMagnitude({
  value,
  median,
  gapPct,
  gapDelta,
  format,
  unit,
}: GapInput): string | null {
  // Percentage metrics diverge in percentage POINTS, not a ratio: a relative
  // "−39%" beside a "90%" median reads as points and misstates the gap, and a
  // multiple ("0.6×") is meaningless. The value and median render as rounded
  // percents beside this gap, and points are the one form readers reconcile
  // by addition — so the spread is computed from the DISPLAYED (rounded)
  // operands, not the raw fractions, to make the on-screen arithmetic close.
  if (format === "percent") {
    const displayGap =
      median != null
        ? Math.round(value) - Math.round(median)
        : Math.round(gapDelta);
    return displayGap === 0 ? null : formatPp(displayGap, 0);
  }
  if (
    median != null &&
    Math.abs(median) > 1e-9 &&
    value / median >= MULTIPLE_THRESHOLD
  ) {
    return formatMultiple(value / median);
  }
  if (gapPct != null) return formatGapPct(gapPct);
  const magnitude = formatMetricValue(Math.abs(gapDelta), format, unit);
  if (/^0(?:\.0)?(?:\D|$)/.test(magnitude)) return null;
  const sign = gapDelta >= 0 ? "+" : "-";
  return `${sign}${magnitude}`;
}
