import type { GroupId, MetricGroup } from "@/lib/insight/groups";
import type { MetricDefinition } from "@/api/metric-definitions-client";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/**
 * What we can say about one part of a person's work, in one period.
 *
 * Three states, not two. A part with no value is either a source that never
 * reaches us or a person who did none of that work, and collapsing them loses
 * the only distinction that separates "connect this" from "nothing happened".
 */
export type PartState = "reads" | "nothing_recorded" | "no_data_reaches_us";

/**
 * The metric keys that have ever produced an observation for this tenant.
 *
 * Read from the definition listing, which reports availability rather than
 * filtering it: a definition that is disabled, schema-broken or has never
 * observed anything is still listed, and says so. That makes it the authority
 * on whether a source reaches us at all.
 *
 * This deliberately does NOT infer reachability from nobody in view having a
 * value. A viewer whose visible set is small may see no user of a system that
 * is connected and busy elsewhere, and the smaller their reach the more often
 * that happens — the same shape of error as a statistic drawn from a truncated
 * pool. Reachability is a property of the tenant, so it is read from the one
 * place that holds it for the tenant.
 *
 * NOT a duplicate of `useAvailableMetricKeys`, which reads the same listing to
 * a different question: that one asks which metrics may be REQUESTED and gates
 * on `is_enabled` alone. This asks which have ever ANSWERED. The gap between
 * the two is the interesting case — a metric that is enabled and requestable
 * but has never observed anything comes back as nulls, and those nulls are
 * what has to be read as "no data reaches us" rather than as an idle person.
 */
export function reachableMetricKeys(
  definitions: readonly MetricDefinition[],
): Set<string> {
  const out = new Set<string>();
  for (const d of definitions) {
    if (!d.is_enabled) continue;
    if (d.schema_status === "error") continue;
    // A custom metric runs its SQL at query time, and the validator stamps
    // freshness from materialized relations only — so its `last_observed_date`
    // is absent however much data it serves. The listing says so in as many
    // words. Judging it by that field would report a working metric as one
    // nothing reaches, which is the exact fabrication this function exists to
    // avoid, so it is taken at its word instead.
    if (d.origin !== "custom" && d.last_observed_date == null) continue;
    out.add(d.metric_key);
  }
  return out;
}

/**
 * Which of the three states a part is in for one person.
 *
 * Order matters: a value settles it, and only in its absence does the question
 * become whose absence it is. A part counts as reaching us when ANY of its
 * metrics does — a section is not unobservable because one of its four metrics
 * is still unwired.
 */
export function partState(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string,
  reachable: ReadonlySet<string>,
): PartState {
  let anyReachable = false;
  for (const m of def.collection.metrics) {
    const metric = byKey.get(m.key);
    if (metric != null && forEntity(metric, entityId).value != null) {
      return "reads";
    }
    if (reachable.has(m.key)) anyReachable = true;
  }
  return anyReachable ? "nothing_recorded" : "no_data_reaches_us";
}

export interface UnreachablePart {
  id: GroupId;
  title: string;
}

/**
 * The parts no metric of which reaches this tenant.
 *
 * What this deliberately does not do is say how many people connecting one
 * would light up. Nobody knows: the people who do that work are invisible
 * precisely because the source is missing, so any such number would be
 * invented. The honest statement is which parts nobody is measured in, next to
 * how many people are thinly covered — and the reader draws the conclusion.
 */
export function unreachableParts(
  groups: readonly MetricGroup[],
  reachable: ReadonlySet<string>,
): UnreachablePart[] {
  return groups
    .filter((def) => !def.collection.metrics.some((m) => reachable.has(m.key)))
    .map((def) => ({ id: def.id, title: def.title }));
}
