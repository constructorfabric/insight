import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  attentionSummary,
  computeAttentionFlags,
  type FlagParams,
} from "./attention-flags";

function fixture(
  period: Array<[string, number | null]>,
  opts?: Partial<Pick<NormalizedMetricResult, "direction" | "label" | "format">>,
): NormalizedMetricResult {
  return {
    metric_key: "t.metric",
    label: opts?.label ?? "Commits",
    unit: null,
    computation: "sum",
    format: opts?.format ?? "integer",
    direction: opts?.direction ?? "higher_is_better",
    period: {
      view: "period",
      values: period.map(([entity_id, value]) => ({ entity_id, value })),
    },
  } as unknown as NormalizedMetricResult;
}

/** 7 well-behaved members around 10, plus one member under test. */
const BASE: Array<[string, number]> = [
  ["m1", 9], ["m2", 10], ["m3", 10], ["m4", 11], ["m5", 10], ["m6", 9], ["m7", 11],
];
const IDS = [...BASE.map(([id]) => id), "x"];

function params(over: Partial<FlagParams>): FlagParams {
  return {
    headlineKeys: ["t.metric"],
    byKey: new Map(),
    previousByKey: new Map(),
    memberIds: IDS,
    cohortOf: () => "team",
    nameOf: (id) => `Name ${id}`,
    personIdOf: (id) => `person-${id}`,
    cohortLabel: "team",
    ...over,
  };
}

describe("computeAttentionFlags", () => {
  it("flags a collapse when a member has zero against a positive median", () => {
    const flags = computeAttentionFlags(
      params({ byKey: new Map([["t.metric", fixture([...BASE, ["x", 0]])]]) }),
    );
    expect(flags).toHaveLength(1);
    expect(flags[0]).toMatchObject({
      kind: "collapse",
      personId: "person-x",
      name: "Name x",
    });
    expect(flags[0]!.reason).toContain("no commits");
    expect(flags[0]!.reason).toContain("team median 10");
  });

  it("flags a low outlier below the Tukey fence on a higher-is-better metric", () => {
    const flags = computeAttentionFlags(
      params({ byKey: new Map([["t.metric", fixture([...BASE, ["x", 2]])]]) }),
    );
    expect(flags).toHaveLength(1);
    expect(flags[0]!.kind).toBe("outlier");
    expect(flags[0]!.reason).toContain("unusually low");
  });

  it("flags a HIGH outlier when lower is better (e.g. meeting hours)", () => {
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([
          ["t.metric", fixture([...BASE, ["x", 40]], { direction: "lower_is_better" })],
        ]),
      }),
    );
    expect(flags).toHaveLength(1);
    expect(flags[0]!.kind).toBe("outlier");
    expect(flags[0]!.reason).toContain("unusually high");
  });

  it("does not flag anyone in a tight, healthy cohort", () => {
    const flags = computeAttentionFlags(
      params({ byKey: new Map([["t.metric", fixture([...BASE, ["x", 10]])]]) }),
    );
    expect(flags).toEqual([]);
  });

  it("skips neutral-direction metrics entirely", () => {
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([
          ["t.metric", fixture([...BASE, ["x", 0]], { direction: "neutral" })],
        ]),
      }),
    );
    expect(flags).toEqual([]);
  });

  it("stays silent when the cohort is too small to judge", () => {
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([["t.metric", fixture([["m1", 10], ["m2", 10], ["x", 0]])]]),
        memberIds: ["m1", "m2", "x"],
      }),
    );
    expect(flags).toEqual([]);
  });

  it("flags an adverse period-over-period decline", () => {
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([["t.metric", fixture([...BASE, ["x", 8]])]]),
        previousByKey: new Map([["t.metric", fixture([...BASE, ["x", 16]])]]),
      }),
    );
    expect(flags).toHaveLength(1);
    expect(flags[0]!.kind).toBe("decline");
    expect(flags[0]!.reason).toBe("down 50% vs last period");
  });

  it("treats an INCREASE as adverse when lower is better", () => {
    // x=12 stays inside the Tukey fence (so the outlier branch, which is
    // checked first and wins, stays quiet) but is +50% vs last period.
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([
          ["t.metric", fixture([...BASE, ["x", 12]], { direction: "lower_is_better" })],
        ]),
        previousByKey: new Map([
          ["t.metric", fixture([...BASE, ["x", 8]], { direction: "lower_is_better" })],
        ]),
      }),
    );
    expect(flags).toHaveLength(1);
    expect(flags[0]!.kind).toBe("decline");
    expect(flags[0]!.reason).toBe("up 50% vs last period");
  });

  it("judges members within their own cohort, not the whole roster", () => {
    // Cohort A hovers at 10; cohort B at 1000. x belongs to B with 900 —
    // fine within B, catastrophic vs the global median. Must NOT flag.
    const period: Array<[string, number]> = [
      ["a1", 9], ["a2", 10], ["a3", 11], ["a4", 10],
      ["b1", 990], ["b2", 1000], ["b3", 1010], ["b4", 1000], ["x", 995],
    ];
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([["t.metric", fixture(period)]]),
        memberIds: period.map(([id]) => id),
        cohortOf: (id) => (id.startsWith("a") ? "A" : "B"),
      }),
    );
    expect(flags).toEqual([]);
  });

  it("keeps only the strongest flag per person+metric and ranks by severity", () => {
    // x collapses (0 vs median) AND declined vs last period — collapse wins.
    const flags = computeAttentionFlags(
      params({
        byKey: new Map([["t.metric", fixture([...BASE, ["x", 0], ["y", 2]])]]),
        previousByKey: new Map([["t.metric", fixture([...BASE, ["x", 10], ["y", 2]])]]),
        memberIds: [...IDS, "y"],
      }),
    );
    const x = flags.filter((f) => f.personId === "person-x");
    expect(x).toHaveLength(1);
    expect(x[0]!.kind).toBe("collapse");
    // collapse severity (1 + relGap) outranks y's outlier severity (relGap)
    expect(flags[0]!.personId).toBe("person-x");
  });
});

describe("attentionSummary", () => {
  it("reports steady when there are no flags", () => {
    expect(attentionSummary([], 0, 12)).toBe(
      "All 12 people are within their usual range this period.",
    );
  });

  it("names the top metric themes with counts", () => {
    const flags = computeAttentionFlags(
      params({ byKey: new Map([["t.metric", fixture([...BASE, ["x", 0]])]]) }),
    );
    expect(attentionSummary(flags, 1, 8)).toBe(
      "1 of 8 people need a look — most flags on Commits (1).",
    );
  });
});

describe("severity is scale-free", () => {
  /** Flags for a single metric over `values`, every value multiplied by `factor`. */
  function flagsFor(values: Array<[string, number]>, factor = 1, cohortOf?: (id: string) => string) {
    const scaled = values.map(([id, v]) => [id, v * factor] as [string, number]);
    return computeAttentionFlags(
      params({
        byKey: new Map([["t.metric", fixture(scaled)]]),
        memberIds: scaled.map(([id]) => id),
        ...(cohortOf ? { cohortOf } : {}),
      }),
    );
  }

  // A cohort with real spread and one member at zero.
  const SPREAD: Array<[string, number]> = [
    ["m1", 10], ["m2", 11], ["m3", 12], ["m4", 13], ["m5", 14], ["m6", 15], ["x", 0],
  ];

  it("ranks identically when every value is scaled by 1000", () => {
    const order = (fs: ReturnType<typeof flagsFor>) => fs.map((f) => f.personId).join(",");
    expect(order(flagsFor(SPREAD, 1000))).toBe(order(flagsFor(SPREAD)));
  });

  it("gives the same severity for the same shape at any magnitude", () => {
    const [small] = flagsFor(SPREAD);
    const [big] = flagsFor(SPREAD, 1000);
    expect(small).toBeDefined();
    expect(big!.severity).toBeCloseTo(small!.severity, 6);
  });

  it("separates collapses instead of tying them at a constant", () => {
    // Two cohorts, each with a zero. The tight cohort's zero sits further out
    // in IQRs, so it must outrank the scattered cohort's zero — under the old
    // constant both scored exactly 2 and the order was arbitrary.
    const values: Array<[string, number]> = [
      ["t1", 10], ["t2", 10], ["t3", 10], ["t4", 11], ["t0", 0],
      ["w1", 2], ["w2", 20], ["w3", 40], ["w4", 60], ["w0", 0],
    ];
    const collapses = flagsFor(values, 1, (id) => (id.startsWith("t") ? "tight" : "wide")).filter(
      (f) => f.kind === "collapse",
    );
    expect(collapses.map((f) => f.personId)).toEqual(["person-t0", "person-w0"]);
    expect(collapses[0]!.severity).toBeGreaterThan(collapses[1]!.severity);
  });

  it("raises nothing for a cohort with no scale at all", () => {
    // Everyone identical and at zero: nothing is unusual, and no scale exists to
    // rank by. A flag here would carry a number that means something else.
    expect(flagsFor([["m1", 0], ["m2", 0], ["m3", 0], ["m4", 0]])).toEqual([]);
  });
});
