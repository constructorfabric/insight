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

export interface PersonCoverage {
  entityId: string;
  /** One entry per part, in the order the parts were given. */
  states: ReadonlyMap<GroupId, PartState>;
  /** How many parts read. This is the coverage level. */
  level: number;
}

export function personCoverage(
  groups: readonly MetricGroup[],
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string,
  reachable: ReadonlySet<string>,
): PersonCoverage {
  const states = new Map<GroupId, PartState>();
  let level = 0;
  for (const def of groups) {
    const state = partState(def, byKey, entityId, reachable);
    states.set(def.id, state);
    if (state === "reads") level += 1;
  }
  return { entityId, states, level };
}

export interface CoverageDistribution {
  /**
   * How many people this count covered. Stated wherever the distribution is,
   * and not optional: a count over the people one viewer can see is a true
   * statement about those people and a false one about the organisation, and
   * this number is the whole difference between the two.
   */
  counted: number;
  /** Level → how many people sit at it. Every level from 0 to `parts` is present. */
  byLevel: ReadonlyMap<number, number>;
}

export function coverageDistribution(
  people: readonly PersonCoverage[],
  parts: number,
): CoverageDistribution {
  const byLevel = new Map<number, number>();
  // Seeded so an empty level reads as zero people rather than as a gap — the
  // shape of the distribution is the finding, and a missing bar hides it.
  for (let level = 0; level <= parts; level += 1) byLevel.set(level, 0);
  for (const p of people) {
    byLevel.set(p.level, (byLevel.get(p.level) ?? 0) + 1);
  }
  return { counted: people.length, byLevel };
}

export interface PartCoverage {
  id: GroupId;
  title: string;
  /** People this part reads for. */
  seen: number;
  /**
   * True when nothing feeding this part reaches the tenant. Kept separate from
   * `seen === 0` on purpose: they render differently because they mean
   * different things, and a part nobody is measured in must not be drawn as a
   * part everybody failed at.
   */
  unreachable: boolean;
}

/**
 * The same coverage, cut by part instead of by person.
 *
 * Derived from the SAME per-person states as the distribution rather than
 * computed its own way. Two counts of one thing on one screen will disagree
 * eventually — different key sets, different fetches, different rounding — and
 * a reader who spots the disagreement is right to stop trusting both.
 */
export function partCoverage(
  groups: readonly MetricGroup[],
  people: readonly PersonCoverage[],
): PartCoverage[] {
  return groups.map((def) => {
    let seen = 0;
    let anyReachable = false;
    for (const p of people) {
      const state = p.states.get(def.id);
      if (state === "reads") seen += 1;
      if (state !== "no_data_reaches_us") anyReachable = true;
    }
    return {
      id: def.id,
      title: def.title,
      seen,
      unreachable: !anyReachable,
    };
  });
}

/**
 * How many people are seen in fewer than half the parts of their work.
 *
 * The one number on this screen that is a finding rather than a description.
 * A distribution shows a shape; what a reader needs from it is whether the
 * rest of the product can bear weight, and for whom it cannot — every metric,
 * comparison and flag about these people rests on that fraction of what they
 * do.
 *
 * "Fewer than half" rather than a tuned threshold: it needs no defending and
 * means the same when the number of parts changes, which a fixed count would
 * not. With an odd number of parts the midpoint is not itself reachable, so
 * the boundary is unambiguous either way.
 */
export function thinlyCovered(
  people: readonly PersonCoverage[],
  parts: number,
): number {
  return people.filter((p) => p.level < parts / 2).length;
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
