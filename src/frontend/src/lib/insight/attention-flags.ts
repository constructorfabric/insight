import { formatMetricValue } from "@/lib/format";
import { MIN_COHORT, quantile } from "@/lib/insight/within-team-peer";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/** Why a person was flagged: apart from their cohort, moving adversely, or at zero. */
export type FlagKind = "outlier" | "decline" | "collapse";

export interface AttentionFlag {
  /** Person id (the identity-cutover key), not an email. */
  personId: string;
  name: string;
  metricKey: string;
  metricLabel: string;
  kind: FlagKind;
  valueText: string;
  reason: string;
  severity: number;
}

/** A member is flagged only when the divergence clears this fraction of the median. */
const OUTLIER_MIN_REL_GAP = 0.25;
/** Adverse period-over-period move that trips a "declining" flag. */
const DELTA_MIN = 0.25;

export interface FlagParams {
  headlineKeys: readonly string[];
  byKey: Map<string, NormalizedMetricResult>;
  previousByKey: Map<string, NormalizedMetricResult>;
  memberIds: readonly string[];
  /** Cohort key per member — flags are computed within each person's cohort. */
  cohortOf: (id: string) => string | null;
  nameOf: (id: string) => string;
  personIdOf: (id: string) => string;
  /** How the cohort is named in reasons ("team" / "division" / …). */
  cohortLabel: string;
}

/**
 * The shared "needs attention" scan: per headline metric, judge each person
 * against their own cohort (Tukey outliers + period declines + collapses).
 * Used by the team-state roster and the org overview so the logic lives once.
 */
/**
 * Severity in units of the cohort's own spread.
 *
 * One ranked list mixes metrics measured in commits, hours and dollars, so the
 * number that orders them cannot carry a unit. Dividing an adverse distance by
 * the cohort's IQR (or |median| when the cohort is flat) makes the result
 * dimensionless: multiply every value of a metric by 1000 and nothing moves.
 *
 * `null` when the cohort offers no scale — a flag that cannot be compared is
 * not raised, rather than raised with a number that means something else.
 */
function severityIn(scale: number | null, adverseDistance: number): number | null {
  if (scale == null || !(scale > 1e-9)) return null;
  const sev = adverseDistance / scale;
  return Number.isFinite(sev) && sev > 0 ? sev : null;
}

export function computeAttentionFlags({
  headlineKeys,
  byKey,
  previousByKey,
  memberIds,
  cohortOf,
  nameOf,
  personIdOf,
  cohortLabel,
}: FlagParams): AttentionFlag[] {
  const out: AttentionFlag[] = [];
  for (const key of headlineKeys) {
    const r = byKey.get(key);
    if (!r || r.direction === "neutral") continue;
    const prev = previousByKey.get(key);
    const higherIsBetter = r.direction !== "lower_is_better";
    const points = memberIds
      .map((id) => ({ id, v: forEntity(r, id).value, c: cohortOf(id) }))
      .filter(
        (pt): pt is { id: string; v: number; c: string } =>
          pt.v != null && Number.isFinite(pt.v) && pt.c != null,
      );
    if (points.length < MIN_COHORT) continue;

    const byCohort = new Map<string, number[]>();
    for (const pt of points)
      (byCohort.get(pt.c) ?? byCohort.set(pt.c, []).get(pt.c)!).push(pt.v);
    const stats = new Map<
      string,
      {
        p50: number;
        loFence: number;
        hiFence: number;
        iqr: number;
        denom: number;
        /**
         * The unit severity is measured in. IQR first — a robust spread that
         * carries the metric's own scale, so "two IQRs from the median" means
         * the same thing for hours and for commits. |median| is the fallback
         * for a cohort with no dispersion; `null` means the cohort offers no
         * scale at all (everyone identical AND at zero), and a flag with no
         * scale cannot be ranked, so it is not raised.
         */
        scale: number | null;
        medianText: string;
      }
    >();
    for (const [c, vals] of byCohort) {
      if (vals.length < MIN_COHORT) continue;
      const sorted = [...vals].sort((a, b) => a - b);
      const p50 = quantile(sorted, 0.5);
      const p25 = quantile(sorted, 0.25);
      const p75 = quantile(sorted, 0.75);
      const iqr = p75 - p25;
      stats.set(c, {
        p50,
        iqr,
        loFence: p25 - 1.5 * iqr,
        hiFence: p75 + 1.5 * iqr,
        denom: Math.abs(p50) > 1e-9 ? Math.abs(p50) : 1,
        scale: iqr > 1e-9 ? iqr : Math.abs(p50) > 1e-9 ? Math.abs(p50) : null,
        medianText: formatMetricValue(p50, r.format, r.unit),
      });
    }

    for (const { id, v, c } of points) {
      const st = stats.get(c);
      const name = nameOf(id);
      const personId = personIdOf(id);
      const valueText = formatMetricValue(v, r.format, r.unit);
      const label = r.short_label ?? r.label;
      const relGap = st
        ? higherIsBetter
          ? (st.p50 - v) / st.denom
          : (v - st.p50) / st.denom
        : 0;

      if (higherIsBetter && v === 0 && st && st.p50 > 0) {
        // A collapse is the extreme of the same measure, not a separate scale:
        // how far below the cohort this person sits, in IQRs. Ranking them all
        // at a constant made every collapse tie and flood the top of the list
        // — and on a partly-adopted metric everyone at zero is a collapse.
        const sev = severityIn(st.scale, st.p50 - v);
        if (sev != null) {
          out.push({
            personId, name, metricKey: key, metricLabel: label, kind: "collapse",
            valueText, reason: `no ${label.toLowerCase()} (${cohortLabel} median ${st.medianText})`,
            severity: sev,
          });
        }
        continue;
      }
      if (
        st &&
        st.iqr > 1e-9 &&
        (higherIsBetter ? v < st.loFence : v > st.hiFence) &&
        relGap >= OUTLIER_MIN_REL_GAP
      ) {
        const sev = severityIn(st.scale, higherIsBetter ? st.p50 - v : v - st.p50);
        if (sev != null) {
          out.push({
            personId, name, metricKey: key, metricLabel: label, kind: "outlier",
            valueText,
            reason: `${higherIsBetter ? "unusually low" : "unusually high"} · ${cohortLabel} median ${st.medianText}`,
            severity: sev,
          });
        }
        continue;
      }
      if (prev) {
        const pv = forEntity(prev, id).value;
        if (pv != null && Number.isFinite(pv) && Math.abs(pv) > 1e-9) {
          const change = (v - pv) / Math.abs(pv);
          const adverse = higherIsBetter ? -change : change;
          if (adverse >= DELTA_MIN) {
            // The trigger stays a percentage of the person's own past — that is
            // the question a decline answers. Severity converts the MOVE into
            // cohort IQRs so it can share a ranking with the other two kinds;
            // without a cohort scale there is nothing to compare against, so
            // fall back to the fractional change and keep the flag.
            const moved = higherIsBetter ? pv - v : v - pv;
            const sev = st ? severityIn(st.scale, moved) : null;
            out.push({
              personId, name, metricKey: key, metricLabel: label, kind: "decline",
              valueText,
              reason: `${higherIsBetter ? "down" : "up"} ${Math.round(adverse * 100)}% vs last period`,
              severity: sev ?? adverse,
            });
          }
        }
      }
    }
  }
  // Strongest flag per (person, metric); ranked by severity.
  const best = new Map<string, AttentionFlag>();
  for (const f of out) {
    const k = `${f.personId}::${f.metricKey}`;
    const cur = best.get(k);
    if (!cur || f.severity > cur.severity) best.set(k, f);
  }
  return [...best.values()].sort((a, b) => b.severity - a.severity);
}

/** "1 person" / "3 people" — a scope of one is a real case (a lead with one report). */
function people(n: number): string {
  return `${n} ${n === 1 ? "person" : "people"}`;
}

/** Deterministic one-liner over the flag set — placeholder for a future AI insight. */
export function attentionSummary(
  flags: AttentionFlag[],
  flaggedPeople: number,
  teamSize: number,
): string {
  if (flags.length === 0)
    return `All ${people(teamSize)} are within their usual range this period.`;
  const byMetric = new Map<string, number>();
  for (const f of flags) byMetric.set(f.metricLabel, (byMetric.get(f.metricLabel) ?? 0) + 1);
  const top = [...byMetric.entries()].sort((a, b) => b[1] - a[1]).slice(0, 2);
  const themes = top.map(([label, n]) => `${label} (${n})`).join(", ");
  return `${flaggedPeople} of ${people(teamSize)} need a look — most flags on ${themes}.`;
}
