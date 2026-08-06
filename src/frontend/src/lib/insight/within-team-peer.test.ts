import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { forEntity } from "@/lib/metrics/collection";
import {
  injectCohortPeer,
  MIN_COHORT,
  quantile,
  withinCohortPeer,
  withinTeamPeer,
} from "./within-team-peer";

function fixture(period: Array<[string, number | null]>): NormalizedMetricResult {
  return {
    metric_key: "t.metric",
    label: "T",
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: period.map(([entity_id, value]) => ({ entity_id, value })),
    },
  } as unknown as NormalizedMetricResult;
}

describe("quantile", () => {
  it("returns NaN on empty input", () => expect(quantile([], 0.5)).toBeNaN());
  it("returns the single element regardless of q", () =>
    expect(quantile([7], 0.9)).toBe(7));
  it("interpolates the median of an even-length array", () =>
    expect(quantile([1, 2, 3, 4], 0.5)).toBe(2.5));
  it("interpolates p25 linearly", () =>
    // pos = 3 * 0.25 = 0.75 → 1 + (2-1) * 0.75 = 1.75
    expect(quantile([1, 2, 3, 4], 0.25)).toBe(1.75));
  it("hits exact ends", () => {
    expect(quantile([1, 2, 3], 0)).toBe(1);
    expect(quantile([1, 2, 3], 1)).toBe(3);
  });
});

describe("withinTeamPeer", () => {
  it("computes median/p25/p75 for every member from the whole roster", () => {
    const ids = ["a", "b", "c", "d"];
    const r = withinTeamPeer(fixture([["a", 1], ["b", 2], ["c", 3], ["d", 4]]), ids);
    const a = forEntity(r, "a").peer!;
    expect(a.median).toBe(2.5);
    expect(a.p25).toBe(1.75);
    expect(a.p75).toBe(3.25);
    expect(a.n).toBe(4);
    // target_value stays the member's own reading
    expect(a.target_value).toBe(1);
    expect(forEntity(r, "d").peer!.target_value).toBe(4);
  });

  it("gives no stats when fewer than MIN_COHORT values are measured", () => {
    const ids = ["a", "b", "c", "d"];
    // only 3 finite values — below MIN_COHORT
    const r = withinTeamPeer(fixture([["a", 1], ["b", 2], ["c", 3], ["d", null]]), ids);
    expect(MIN_COHORT).toBe(4);
    expect(forEntity(r, "a").peer!.median).toBeNull();
    expect(forEntity(r, "a").peer!.n).toBe(0);
  });
});

describe("withinCohortPeer", () => {
  const ids = ["a1", "a2", "a3", "a4", "b1", "b2"];
  const r = fixture([
    ["a1", 10], ["a2", 20], ["a3", 30], ["a4", 40],
    ["b1", 1000], ["b2", 2000],
  ]);
  const cohortOf = (id: string) => (id.startsWith("a") ? "A" : "B");

  it("ranks each member against their OWN cohort only", () => {
    const out = withinCohortPeer(r, ids, cohortOf);
    // cohort A median = 25, untouched by B's thousands
    expect(forEntity(out, "a1").peer!.median).toBe(25);
    expect(forEntity(out, "a1").peer!.min).toBe(10);
    expect(forEntity(out, "a1").peer!.max).toBe(40);
  });

  it("suppresses small cohorts instead of faking a comparison", () => {
    const out = withinCohortPeer(r, ids, cohortOf);
    // cohort B has 2 members < MIN_COHORT → neutral
    expect(forEntity(out, "b1").peer!.median).toBeNull();
    expect(forEntity(out, "b1").peer!.n).toBe(0);
    // but the member still carries their own value
    expect(forEntity(out, "b1").peer!.target_value).toBe(1000);
  });

  it("excludes members whose cohort resolves to null", () => {
    const out = withinCohortPeer(r, ids, (id) => (id === "a1" ? null : cohortOf(id)));
    // a1 excluded → cohort A has 3 measured values < MIN_COHORT → suppressed
    expect(forEntity(out, "a2").peer!.median).toBeNull();
  });

  it("does not mutate the input result", () => {
    withinCohortPeer(r, ids, cohortOf);
    expect(r.peer).toBeUndefined();
  });
});

describe("injectCohortPeer", () => {
  const cohortIds = ["a", "b", "c", "d"];
  const cohortR = fixture([["a", 1], ["b", 2], ["c", 3], ["d", 4]]);
  const personR = fixture([["a", 1]]);

  it("overlays the person's result with cohort-derived peer stats", () => {
    const out = injectCohortPeer(
      new Map([["t.metric", personR]]),
      new Map([["t.metric", cohortR]]),
      cohortIds,
    );
    const merged = out.get("t.metric")!;
    expect(forEntity(merged, "a").peer!.median).toBe(2.5);
    // the person's own period view is preserved
    expect(forEntity(merged, "a").value).toBe(1);
  });

  it("keeps the person's result untouched when the cohort lacks that metric", () => {
    const out = injectCohortPeer(
      new Map([["t.metric", personR]]),
      new Map(),
      cohortIds,
    );
    expect(out.get("t.metric")).toBe(personR);
  });

  it("is a no-op on an empty cohort", () => {
    const byKey = new Map([["t.metric", personR]]);
    expect(injectCohortPeer(byKey, new Map([["t.metric", cohortR]]), [])).toBe(byKey);
  });
});
