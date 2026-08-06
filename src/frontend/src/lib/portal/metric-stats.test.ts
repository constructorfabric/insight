import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  chooseStep,
  distribution,
  familyObserved,
  fmtCompact,
  groupCoverage,
  medianAcross,
  perCapita,
  representative,
  topDecileShare,
} from "./metric-stats";

/** Minimal NormalizedMetricResult fixture: period values + peer target_values. */
function fixture(opts: {
  computation?: NormalizedMetricResult["computation"];
  period: Array<[string, number | null]>;
  peerTargets?: Array<[string, number | null]>;
}): NormalizedMetricResult {
  return {
    metric_key: "t.metric",
    label: "T",
    unit: null,
    computation: opts.computation ?? "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: opts.period.map(([entity_id, value]) => ({ entity_id, value })),
    },
    peer: opts.peerTargets
      ? {
          view: "peer",
          values: opts.peerTargets.map(([entity_id, target_value]) => ({
            entity_id,
            target_value,
          })),
        }
      : undefined,
  } as unknown as NormalizedMetricResult;
}

describe("chooseStep", () => {
  it("keeps small maxima at step 1", () => expect(chooseStep(6, 14)).toBe(1));
  it("climbs the 1/2/5 ladder", () => expect(chooseStep(70, 14)).toBe(5));
  it("scales into hundreds", () => expect(chooseStep(4500, 14)).toBe(500));
});

describe("distribution", () => {
  it("suppresses below 4 observations", () =>
    expect(distribution([1, 2, 3], String)).toEqual([]));
  it("suppresses when all mass lands in one bin", () =>
    expect(distribution([5, 5, 5, 5], String)).toEqual([]));
  it("bins a real spread", () => {
    const rows = distribution([1, 2, 3, 9], String);
    expect(rows.length).toBeGreaterThan(1);
    expect(rows.reduce((a, r) => a + r.count, 0)).toBe(4);
  });
});

describe("topDecileShare", () => {
  it("needs at least 4 values", () => expect(topDecileShare([1, 2, 3])).toBeNull());
  it("computes the busiest-decile share", () => {
    const share = topDecileShare([10, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
    expect(share).toBeCloseTo(10 / 19, 3);
  });
});

describe("perCapita / representative", () => {
  it("perCapita divides by ACTIVE people only", () => {
    const r = fixture({ period: [["a", 10], ["b", 0], ["c", 20]] });
    expect(perCapita(r, ["a", "b", "c"])).toBe(15);
  });
  it("representative sums counters and medians ratios", () => {
    const sum = fixture({ period: [["a", 1], ["b", 2]] });
    expect(representative(sum, ["a", "b"])).toBe(3);
    const med = fixture({ computation: "ratio", period: [["a", 10], ["b", 30]] });
    expect(representative(med, ["a", "b"])).toBe(20);
  });
});

describe("familyObserved", () => {
  it("false when every peer target is null (zero-filled sums don't count)", () => {
    const r = fixture({ period: [["a", 0]], peerTargets: [["a", null]] });
    expect(familyObserved(new Map([["t.metric", r]]), ["t.metric"], ["a"])).toBe(false);
  });
  it("true when any entity is observed", () => {
    const r = fixture({ period: [["a", 0], ["b", 7]], peerTargets: [["a", null], ["b", 7]] });
    expect(familyObserved(new Map([["t.metric", r]]), ["t.metric"], ["a", "b"])).toBe(true);
  });
});

describe("medianAcross", () => {
  it("medians a summable metric instead of summing it (unlike representative)", () => {
    const r = fixture({ computation: "sum", period: [["a", 10], ["b", 20], ["c", 30]] });
    expect(medianAcross(r, ["a", "b", "c"])).toBe(20);
    expect(representative(r, ["a", "b", "c"])).toBe(60);
  });
  it("null when the metric is missing", () => expect(medianAcross(undefined, ["a"])).toBeNull());
});

describe("fmtCompact", () => {
  it("abbreviates thousands", () => expect(fmtCompact(1500)).toBe("1.5k"));
  it("keeps small integers", () => expect(fmtCompact(10)).toBe("10"));
  // Without the million step a large bin edge read "1000k", which names a
  // magnitude the reader has to decode.
  it("abbreviates millions", () => {
    expect(fmtCompact(1_500_000)).toBe("1.5M");
    expect(fmtCompact(2_000_000)).toBe("2M");
  });
});

describe("groupCoverage", () => {
  it("counts only entityObserved members (zero-filled sums excluded)", () => {
    const r = fixture({
      period: [["a", 0], ["b", 7], ["c", 3]],
      peerTargets: [["a", null], ["b", 7], ["c", 3]],
    });
    const byKey = new Map([["t.metric", r]]);
    expect(groupCoverage(byKey, ["t.metric"], ["a", "b", "c"])).toBeCloseTo(2 / 3, 5);
  });
  it("returns null for an empty roster", () => {
    expect(groupCoverage(new Map(), ["t.metric"], [])).toBeNull();
  });
  it("returns 0 when the group has no observations", () => {
    const r = fixture({ period: [["a", 0]], peerTargets: [["a", null]] });
    expect(groupCoverage(new Map([["t.metric", r]]), ["t.metric"], ["a"])).toBe(0);
  });
});
