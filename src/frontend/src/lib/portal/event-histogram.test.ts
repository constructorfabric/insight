import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { mergeEventHistogram } from "./event-histogram";

const result = (
  values: Array<{ entity_id: string; bins: Array<{ lo: number; hi: number; count: number }> }>,
) => ({ metric_key: "m", histogram: { view: "histogram", values } }) as unknown as NormalizedMetricResult;

describe("mergeEventHistogram", () => {
  it("sums counts when bin edges align across entities", () => {
    const r = result([
      { entity_id: "a", bins: [{ lo: 0, hi: 1, count: 2 }, { lo: 1, hi: 2, count: 1 }] },
      { entity_id: "b", bins: [{ lo: 0, hi: 1, count: 3 }, { lo: 1, hi: 2, count: 0 }] },
    ]);
    expect(mergeEventHistogram(r, ["a", "b"])).toEqual([
      { lo: 0, hi: 1, count: 5 },
      { lo: 1, hi: 2, count: 1 },
    ]);
  });
  it("returns null when edges differ (per-entity bins are not summable)", () => {
    const r = result([
      { entity_id: "a", bins: [{ lo: 0, hi: 1, count: 2 }] },
      { entity_id: "b", bins: [{ lo: 0, hi: 5, count: 3 }] },
    ]);
    expect(mergeEventHistogram(r, ["a", "b"])).toBeNull();
  });
  it("returns null when nothing is there", () => {
    expect(mergeEventHistogram(undefined, ["a"])).toBeNull();
  });
});
