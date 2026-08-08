import type { MetricGroup } from "@/lib/insight/groups";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/**
 * Dimension keys that name a system rather than a property of the work.
 *
 * `tool` and `source` say which product the reading came out of. `scope`
 * (internal / external), `type`, `category` and the rest describe the thing
 * being counted, not where it was observed, and putting them in the same
 * sentence would claim a connector that does not exist.
 */
const SYSTEM_DIMENSIONS = new Set(["tool", "source"]);

/**
 * The systems this section's numbers actually came out of, named.
 *
 * A section says how much someone collaborated; it should also say what it
 * was watching when it decided that. Without the line, a reader has no way to
 * tell a low number that means "little happened" from one that means "the
 * chat tool everybody uses is not connected" — and the second is not their
 * problem to answer for.
 *
 * Built from breakdowns the section already fetched, so it costs nothing.
 * Only systems that produced something are listed: a tool with a zero reading
 * is either unused by this person or unwired, and this line cannot tell those
 * apart — so it does not claim to.
 */
export function sectionSources(
  def: MetricGroup,
  byKey: Map<string, NormalizedMetricResult>,
  entityId: string
): string[] {
  const names = new Set<string>();
  for (const m of def.collection.metrics) {
    const metric = byKey.get(m.key);
    if (!metric) continue;
    for (const row of forEntity(metric, entityId).breakdown) {
      if ((row.value ?? 0) <= 0) continue;
      for (const dimension of row.dimensions) {
        if (!SYSTEM_DIMENSIONS.has(dimension.key)) continue;
        const name = dimension.label ?? dimension.value;
        if (name) names.add(name);
      }
    }
  }
  return [...names].sort((a, b) => a.localeCompare(b));
}
