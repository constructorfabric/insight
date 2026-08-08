import { formatMetricValue, splitMetricValue } from "@/lib/format";
import { formatGapMagnitude } from "@/lib/metrics/gap";
import type { MetricGroup, GroupId } from "@/lib/insight/groups";
import { metricHelp, type MetricHelpText } from "@/lib/insight/metric-help";
import { dropRedundantMetrics } from "@/lib/insight/metric-containment";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { derivePeerStanding, peerSpread } from "@/lib/metrics/peer-standing";

/**
 * One "needs attention" row: a metric sitting in the bottom quartile of its
 * cohort, display-ready. Ranking (`relGap` descending) happens in the
 * component.
 */
/**
 * How far a metric must move against its own past to count as a change rather
 * than noise, measured in cohort spreads.
 *
 * The previous rule was a share of the person's own previous value, which made
 * the threshold easiest to clear exactly where the evidence is thinnest: two
 * of something falling to one is a 50% collapse and one event of difference.
 * A quarter of the spread asks instead whether the person moved by an amount
 * that separates people in this cohort at all — the same yardstick the
 * ordering uses, so selection and ranking can no longer disagree.
 */
const ADVERSE_MOVE_MIN_SPREADS = 0.25;

/**
 * How far below the cohort a metric must sit to be worth naming even when it
 * has not moved: a quarter of what a typical peer does, or less.
 *
 * The movement rule alone hid the worst finding a person can have. Someone
 * whose value is essentially zero against a cohort median in the thousands
 * fell there once — possibly before any period on screen — and has sat there
 * since, so a rule that only reports changes reports nothing about them, and
 * reports it on an empty screen that reads as "all clear".
 *
 * Distance is measured against the MEDIAN, not the spread, because this is the
 * regime where the spread lies: a cohort whose top quartile is enormous makes
 * being at zero look like a small step, when it is the whole distance there is.
 */
const BEHIND_MIN_REL_GAP = 0.75;

/**
 * How much a typical peer must be doing before "far behind" means anything,
 * for metrics counted in whole events.
 *
 * Zero files shared against a cohort median of one file is a hundred-percent
 * gap and no finding at all: the cohort barely does the thing either, so the
 * metric separates nobody. Without this the rows that shout loudest are the
 * ones with the least behind them, because a share of a tiny number is
 * always large.
 *
 * A fall is unaffected — that claim rests on the person's own movement, not
 * on the cohort having somewhere to fall from.
 */
const BEHIND_MIN_MEDIAN_EVENTS = 5;

export interface AttentionItem {
  key: string;
  group: GroupId;
  label: string;
  valueText: string;
  /** The number alone, and its unit — so a list can align digits on the right
   *  and units on the left, and have both columns start together. */
  valueNumber: string;
  valueUnit: string;
  /** Formatted peer-median value only (no label); the view frames it. */
  medianText: string | null;
  /** Scale of divergence from the median ("16×", "−40%"); null at the median. */
  gapText: string | null;
  /** The catalog's own words for the metric; null when it supplies none. */
  help: MetricHelpText | null;
  /**
   * Ordering weight: how far below the cohort the metric sits, in spreads.
   * Percent-of-median ordering used to put the smallest counters on top.
   */
  spreadGap: number;
  /** Tiebreak only — two metrics equally far out in spreads. */
  relGap: number;
  /**
   * Why the row is here. "fell" is news — it moved against the person's own
   * past this period. "behind" is a state: far below the cohort and staying
   * there. Naming which is which is the difference between "look at this
   * change" and "this has been true for a while", and a reader cannot tell
   * them apart from a number alone.
   */
  kind: "fell" | "behind";
  /** True when the previous period holds no value to compare against. */
  noPrevious: boolean;
}

/**
 * Metric-collection results → attention items; direction rides the wire.
 *
 * Only metrics the section card does NOT already show. A card carries its
 * `preview` rows in the same colours and a "N behind peers" badge over them, so
 * repeating those rows above put every such finding on the screen twice — and
 * a reader counts red marks, not facts. Two numbers for one problem read as two
 * problems, and a handful of findings can reach the screen as twice as many
 * marks.
 *
 * What is left is what the cards cannot tell you: a metric outside the preview
 * that is nonetheless bottom-quartile. That is the whole job of this block —
 * the thing you would otherwise miss.
 *
 * The card's SUMMARY line counts as shown too. It is the most prominent thing
 * on the card and it is chosen from every metric of the group, not from the
 * three the card lists — so excluding only the preview left the lead metric
 * appearing twice — a card's headline could also be the first attention row.
 */
export function metricAttentionItems(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  previousByKey: Map<string, NormalizedMetricResult> | null,
  entityId: string,
  /**
   * The keys the headline row actually RENDERED. Not `KPI_ROW`: that is a
   * candidate list longer than the row, so excluding all of it hides a
   * bottom-quartile metric that never made it into a slot — it would appear
   * neither above nor here.
   */
  headlineKeys: ReadonlySet<string>
): AttentionItem[] {
  const items: AttentionItem[] = [];

  for (const metricConfig of def.collection.metrics) {
    if (headlineKeys.has(metricConfig.key)) continue;
    const metric = byKey.get(metricConfig.key);
    if (!metric) continue;
    const data = forEntity(metric, entityId);
    const value = data.value;
    // One judgment layer, the same one the tile above this block reads.
    // Deciding "bottom quartile" here from raw percentiles skipped every
    // eligibility guard it owns — most visibly a cohort with no spread at
    // all, where the quartile test ranks somebody bottom for being one of
    // several identical numbers.
    const standing = derivePeerStanding(metric.direction, data);
    if (!standing.eligible || standing.rank !== "bottom") continue;
    if (value == null || !Number.isFinite(value)) continue;
    const stats = standing.stats;
    if (!stats) continue;
    const higherIsBetter = metric.direction !== "lower_is_better";
    // Below the cohort AND moving the wrong way.
    //
    // "Below the cohort" alone is a standing, not an event: a lead measured
    // against the developers reporting to them is below on commits every
    // month, by the shape of the job, and a block that repeats that forever
    // teaches the reader to skip it. A structural gap is flat; a regression is
    // not, so requiring the move keeps the standing out and lets the change
    // through.
    //
    // No previous value makes no claim about direction — such a metric can
    // still be here, but only as a state, never as a fall.
    const before = previousByKey?.get(metricConfig.key);
    const previous = before ? forEntity(before, entityId).value : null;
    const hasPrevious = previous != null && Number.isFinite(previous);

    const median = stats.p50;
    const denom = Math.abs(median) > 1e-9 ? Math.abs(median) : 1;
    const relGap = higherIsBetter
      ? (median - value) / denom
      : (value - median) / denom;

    const fell =
      hasPrevious &&
      (higherIsBetter ? value < previous : value > previous) &&
      Math.abs(value - previous) / peerSpread(stats) >=
        ADVERSE_MOVE_MIN_SPREADS;
    const behind =
      relGap >= BEHIND_MIN_REL_GAP &&
      (metric.format !== "integer" ||
        Math.abs(median) >= BEHIND_MIN_MEDIAN_EVENTS);
    if (!fell && !behind) continue;
    const gapDelta = value - median;
    const split = splitMetricValue(value, metric.format, metric.unit);
    items.push({
      key: metric.metric_key,
      group: def.id,
      label: metric.label,
      valueText: formatMetricValue(value, metric.format, metric.unit),
      valueNumber: split.number,
      valueUnit: split.unit,
      medianText: formatMetricValue(median, metric.format, metric.unit),
      gapText: formatGapMagnitude({
        value,
        median,
        gapPct: Math.abs(median) > 1e-9 ? gapDelta / Math.abs(median) : null,
        gapDelta,
        format: metric.format,
        unit: metric.unit,
      }),
      help: metricHelp(metric),
      spreadGap: standing.spreadGap,
      relGap,
      // A fall is the more specific claim, so it wins when both hold: the
      // reader learns it moved AND sees how far out it is in the same row.
      kind: fell ? "fell" : "behind",
      noPrevious: !hasPrevious,
    });
  }
  return items;
}

/**
 * The block's final list: ranked, then thinned.
 *
 * Ranking first is what makes the thinning honest — of two rows saying the
 * same thing, the one that survives is the one that says it more strongly,
 * not whichever group happened to be evaluated first.
 *
 * A state before a fall. A metric sitting at a fraction of what its peers do
 * is the larger claim, and the block used to be unable to make it at all; a
 * fall of a fraction of the cohort's spread is news, but smaller news. Within
 * each, furthest out first — in spreads for a fall, since that is what
 * qualified it, and in distance from the median for a state, since a wide
 * cohort makes being at zero look like a small step in spreads.
 */
export function orderAttentionItems(
  items: readonly AttentionItem[],
  headlineKeys: ReadonlySet<string>,
  /**
   * Put what CHANGED first — for a reader looking at their own page.
   *
   * A standing is the larger claim and leads by default, which is what a
   * manager scanning someone else needs. On your own page the order inverts:
   * what moved this period is what you can still do something about, while
   * what has been true for months you have already lived through.
   *
   * Neither order hides anything — both kinds are always listed.
   */
  changesFirst = false
): AttentionItem[] {
  const leads = (item: AttentionItem) =>
    changesFirst
      ? Number(item.kind === "fell")
      : Number(item.kind === "behind");
  const ranked = [...items].sort(
    (a, b) =>
      leads(b) - leads(a) ||
      (a.kind === "behind" ? b.relGap - a.relGap : b.spreadGap - a.spreadGap) ||
      b.relGap - a.relGap
  );
  return dropRedundantMetrics(ranked, headlineKeys);
}
