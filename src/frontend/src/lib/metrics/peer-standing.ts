import type { MetricDirection } from "@/api/metric-results-client";
import type { EntityMetricData, PeerEntityStats } from "@/lib/metrics/collection";
import {
  peerStatusVsQuartiles,
  type PeerStats,
  type PeerStatusWithNeutral,
} from "@/lib/peers";

/**
 * The single judgment layer for a person's standing against their peer
 * cohort on the unified metric-results path. Every eligibility guard and
 * the quartile-rank judgment are derived HERE, once — display surfaces
 * (KPI tiles, summary cards, group cards, the peer story) only map the
 * result to their own vocabulary. Surfaces re-deriving standings from raw
 * stats is how contradictory verdicts ("Bottom 25%" on one widget, "GOOD"
 * on another, for the same number) crept in.
 */

/** Why a standing is (in)eligible, most specific reason wins. */
type StandingReason =
  /** Rankable — `rank` is meaningful. */
  | "ok"
  /** No period value for the entity. */
  | "no_value"
  /** The metric declares no better/worse direction. */
  | "neutral_direction"
  /** Zero-filled own total with a null peer target: no observations. */
  | "unmeasured"
  /** No usable cohort stats (no peer view, or suppressed percentiles). */
  | "no_stats"
  /** Cohort has zero spread (everyone identical) — ranks nobody. */
  | "flat_pool";

export interface PeerStanding {
  /** False when the peer view marks the person unmeasured. */
  observed: boolean;
  stats: PeerStats | null;
  eligible: boolean;
  reason: StandingReason;
  /** Quartile rank; "neutral" when ineligible. */
  rank: PeerStatusWithNeutral;
  /**
   * Signed arithmetic distance from the median (positive = above). Display
   * renders this next to "from median" so the sign must track position, not
   * favorability — good/bad is `rank`'s (and the color's) job.
   */
  gapDelta: number;
  gapPct: number | null;
  /** Outlier ordering weight; 0 when ineligible. */
  severity: number;
}

export function toPeerStats(row: PeerEntityStats | null): PeerStats | null {
  if (
    row?.p25 == null ||
    row.median == null ||
    row.p75 == null ||
    row.min == null ||
    row.max == null
  ) {
    return null;
  }
  return {
    p25: row.p25,
    p50: row.median,
    p75: row.p75,
    min: row.min,
    max: row.max,
    n: row.n,
  };
}

/** IQR, falling back to range then 1, for severity normalization. */
function peerSpread(stats: PeerStats): number {
  const iqr = Math.abs(stats.p75 - stats.p25);
  if (iqr > 1e-9) return iqr;
  const range = Math.abs(stats.max - stats.min);
  if (range > 1e-9) return range;
  return 1;
}

export function derivePeerStanding(
  direction: MetricDirection,
  data: Pick<EntityMetricData, "value" | "peer">,
): PeerStanding {
  const value = data.value;
  const stats = toPeerStats(data.peer);
  const observed = data.peer == null || data.peer.target_value != null;
  const higherIsBetter = direction !== "lower_is_better";

  const gapDelta = stats != null && value != null ? value - stats.p50 : 0;
  const gapPct =
    stats != null && Math.abs(stats.p50) > 1e-9
      ? gapDelta / Math.abs(stats.p50)
      : null;

  const ineligible = (reason: Exclude<StandingReason, "ok">): PeerStanding => ({
    observed,
    stats,
    eligible: false,
    reason,
    rank: "neutral",
    gapDelta,
    gapPct,
    severity: 0,
  });

  if (value == null || !Number.isFinite(value)) return ineligible("no_value");
  if (direction === "neutral") return ineligible("neutral_direction");
  if (!observed) return ineligible("unmeasured");
  if (stats == null) return ineligible("no_stats");
  if (Math.abs(stats.max - stats.min) <= 1e-9) return ineligible("flat_pool");

  return {
    observed,
    stats,
    eligible: true,
    reason: "ok",
    rank: peerStatusVsQuartiles(value, stats, higherIsBetter),
    gapDelta,
    gapPct,
    severity: Math.abs(gapPct ?? gapDelta / peerSpread(stats)),
  };
}
