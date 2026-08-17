import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { collectionEvidenceTargets } from "@/lib/metrics/evidence-targets";

function metric(
  metricKey: string,
  options: { drillable?: boolean; selection?: boolean } = {}
): NormalizedMetricResult {
  const { drillable = true, selection = true } = options;
  return {
    metric_key: metricKey,
    label: metricKey.toUpperCase(),
    drilldown: drillable ? { granularity: ["event"] } : undefined,
    selection: selection
      ? {
          metric_key: metricKey,
          entity: { type: "person", ids: ["person-1"] },
          period: { from: "2026-07-01", to: "2026-07-31" },
          filters: [],
        }
      : undefined,
  } as unknown as NormalizedMetricResult;
}

function byKeyOf(...metrics: NormalizedMetricResult[]) {
  return new Map(metrics.map((entry) => [entry.metric_key, entry]));
}

describe("collectionEvidenceTargets", () => {
  it("keeps the order the collection declares", () => {
    const out = collectionEvidenceTargets(
      ["b", "a"],
      byKeyOf(metric("a"), metric("b")),
      "person-1"
    );
    expect(out.map((target) => target.selection.metric_key)).toEqual([
      "b",
      "a",
    ]);
  });

  it("names each target by the metric's label", () => {
    const out = collectionEvidenceTargets(["a"], byKeyOf(metric("a")), "p");
    expect(out[0]?.label).toBe("A");
  });

  it("lists a metric once even when two blocks declare it", () => {
    const out = collectionEvidenceTargets(
      ["a", "a"],
      byKeyOf(metric("a")),
      "person-1"
    );
    expect(out).toHaveLength(1);
  });

  it("leaves out what cannot be drilled", () => {
    const out = collectionEvidenceTargets(
      ["a", "b", "c", "missing"],
      byKeyOf(
        metric("a"),
        metric("b", { drillable: false }),
        metric("c", { selection: false })
      ),
      "person-1"
    );
    expect(out.map((target) => target.selection.metric_key)).toEqual(["a"]);
  });

  it("takes the period it was given over the metric's own", () => {
    const range = { from: "2026-08-01", to: "2026-08-31" };
    const out = collectionEvidenceTargets(
      ["a"],
      byKeyOf(metric("a")),
      "person-1",
      range
    );
    expect(out[0]?.selection.period).toEqual(range);
  });
});
