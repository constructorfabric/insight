/**
 * The tenant lens's derived readings are pure math over view rows — tested
 * here without a DOM. The gate constants must mirror silver's is_gate
 * (github__ci_runs.sql): commit-triggered AND decided.
 */
import { describe, expect, it } from "vitest";

import {
  calloutPair,
  cumulativeShares,
  dayHourMatrix,
  decomposeBy,
  dumbbellPairs,
  gateStatsBy,
  halvesComparison,
  hourColumns,
  marginalImpact,
  mean,
  medianOf,
  sampleStddev,
  scatterPoints,
  smallMultiples,
  splitDateRange,
  stackedTrend,
  trailingOutlierDates,
  weeklyVerdicts,
  type DimRow,
  type DimSeries,
} from "./derived";

function runRow(
  pipeline: string,
  trigger: string,
  outcome: string,
  value: number,
  extra: Array<{ key: string; value: string; label?: string }> = []
): DimRow {
  return {
    value,
    dimensions: [
      { key: "pipeline", value: pipeline, label: pipeline },
      { key: "trigger", value: trigger },
      { key: "outcome", value: outcome },
      ...extra,
    ],
  };
}

describe("gateStatsBy", () => {
  it("counts only commit-triggered decided runs toward the gate", () => {
    const rows = [
      runRow("ci", "push", "success", 8),
      runRow("ci", "pull_request", "failure", 2),
      // Not commit-triggered — runs, but never a gate run.
      runRow("ci", "schedule", "success", 5),
      // Not decided — approval walls and cancels decide nothing.
      runRow("ci", "push", "action_required", 3),
      runRow("ci", "push", "cancelled", 4),
    ];
    const [stats] = gateStatsBy(rows, "pipeline");
    expect(stats.runs).toBe(22);
    expect(stats.gateRuns).toBe(10);
    expect(stats.gatePassed).toBe(8);
    expect(stats.gateFailed).toBe(2);
    expect(stats.passRate).toBeCloseTo(80);
  });

  it("sums finer-grouped rows into one group", () => {
    const rows = [
      runRow("ci", "push", "success", 3, [{ key: "repository", value: "a" }]),
      runRow("ci", "push", "success", 4, [{ key: "repository", value: "b" }]),
    ];
    const [stats] = gateStatsBy(rows, "pipeline");
    expect(stats.gatePassed).toBe(7);
  });

  it("counts merge-queue runs in the gate on both sides", () => {
    const [stats] = gateStatsBy(
      [
        runRow("ci", "merge_queue", "success", 3),
        runRow("ci", "merge_queue", "failure", 1),
      ],
      "pipeline"
    );
    expect(stats.gateRuns).toBe(4);
    expect(stats.gatePassed).toBe(3);
    expect(stats.passRate).toBeCloseTo(75);
  });

  it("keeps timed_out in the gate denominator as a failure", () => {
    const [stats] = gateStatsBy([runRow("ci", "push", "timed_out", 5)], "pipeline");
    expect(stats.gateRuns).toBe(5);
    expect(stats.gateFailed).toBe(5);
    expect(stats.passRate).toBe(0);
  });
});

describe("marginalImpact", () => {
  const rows = [
    runRow("flaky", "push", "success", 10),
    runRow("flaky", "push", "failure", 10),
    runRow("shaky", "push", "success", 15),
    runRow("shaky", "pull_request", "failure", 5),
    runRow("solid", "push", "success", 60),
  ];

  it("converts each worst pipeline's failures into passes step by step", () => {
    const impact = marginalImpact(rows);
    expect(impact).not.toBeNull();
    expect(impact?.gateRuns).toBe(100);
    expect(impact?.currentRate).toBeCloseTo(85);
    expect(impact?.steps.map((s) => s.pipelines.at(-1))).toEqual([
      "flaky",
      "shaky",
    ]);
    expect(impact?.steps[0].rate).toBeCloseTo(95);
    expect(impact?.steps[0].delta).toBeCloseTo(10);
    expect(impact?.steps[1].rate).toBeCloseTo(100);
  });

  it("is null when nothing fails or nothing gates", () => {
    expect(marginalImpact([runRow("solid", "push", "success", 9)])).toBeNull();
    expect(marginalImpact([runRow("cron", "schedule", "failure", 9)])).toBeNull();
  });
});

describe("series statistics", () => {
  it("mean and sample stddev behave on the small-n edges", () => {
    expect(mean([])).toBeNull();
    expect(mean([2, 4])).toBe(3);
    expect(sampleStddev([5])).toBeNull();
    expect(sampleStddev([2, 4])).toBeCloseTo(Math.SQRT2);
    expect(medianOf([])).toBeNull();
    expect(medianOf([3, 1, 2])).toBe(2);
    expect(medianOf([4, 1, 3, 2])).toBe(2.5);
  });

  it("flags only drops below the trailing mean minus 2σ", () => {
    const steady = [96, 95, 97, 96, 95, 96].map((value, i) => ({
      date: `2026-03-0${i + 1}`,
      value,
    }));
    expect(trailingOutlierDates(steady)).toEqual([]);
    const withCrash = [...steady, { date: "2026-03-07", value: 60 }];
    expect(trailingOutlierDates(withCrash)).toEqual(["2026-03-07"]);
  });

  it("never flags without enough prior points or spread", () => {
    expect(
      trailingOutlierDates([
        { date: "d1", value: 90 },
        { date: "d2", value: 10 },
      ])
    ).toEqual([]);
    // A perfectly flat prior window has σ = 0 — nothing to be 2σ below.
    expect(
      trailingOutlierDates([
        { date: "d1", value: 90 },
        { date: "d2", value: 90 },
        { date: "d3", value: 90 },
        { date: "d4", value: 10 },
      ])
    ).toEqual([]);
  });
});

describe("decomposeBy / cumulativeShares", () => {
  const rows: DimRow[] = [
    { value: 30, dimensions: [{ key: "outcome", value: "success" }] },
    { value: 10, dimensions: [{ key: "outcome", value: "failure" }] },
    // Finer-grouped rows of the same segment sum.
    {
      value: 20,
      dimensions: [
        { key: "outcome", value: "success" },
        { key: "repository", value: "a" },
      ],
    },
  ];

  it("resolves segments to shares of the total, largest first", () => {
    const segments = decomposeBy(rows, "outcome");
    expect(segments.map((s) => [s.value, s.amount, Math.round(s.share)])).toEqual([
      ["success", 50, 83],
      ["failure", 10, 17],
    ]);
  });

  it("accumulates ranked shares to 100", () => {
    const ranked = cumulativeShares(rows, "outcome");
    expect(ranked[0].rank).toBe(1);
    expect(ranked[0].cumulativeShare).toBeCloseTo(83.33, 1);
    expect(ranked[1].cumulativeShare).toBeCloseTo(100);
  });

  it("is empty when nothing is positive", () => {
    expect(
      decomposeBy([{ value: 0, dimensions: [{ key: "outcome", value: "x" }] }], "outcome")
    ).toEqual([]);
  });
});

function series(
  dims: Array<{ key: string; value: string; label?: string }>,
  points: Array<[string, number | null]>
): DimSeries {
  return {
    dimensions: dims,
    points: points.map(([bucket_start, value]) => ({ bucket_start, value })),
  };
}

describe("stackedTrend", () => {
  const input = [
    series([{ key: "outcome", value: "success" }], [
      ["2026-03-02", 6],
      ["2026-03-09", 2],
    ]),
    series([{ key: "outcome", value: "failure" }], [
      ["2026-03-02", 2],
      ["2026-03-09", 2],
    ]),
  ];

  it("keys each bucket by segment", () => {
    const { segments, rows } = stackedTrend(input, "outcome");
    expect(segments.map((s) => s.value)).toEqual(["success", "failure"]);
    expect(rows).toEqual([
      { date: "2026-03-02", values: { success: 6, failure: 2 } },
      { date: "2026-03-09", values: { success: 2, failure: 2 } },
    ]);
  });

  it("share mode converts each bucket to percent of its own total", () => {
    const { rows } = stackedTrend(input, "outcome", { share: true });
    expect(rows[0].values).toEqual({ success: 75, failure: 25 });
    expect(rows[1].values).toEqual({ success: 50, failure: 50 });
  });
});

describe("smallMultiples", () => {
  it("ranks by volume, caps to top, and shares one ceiling", () => {
    const input = [
      series([{ key: "repository", value: "big" }], [
        ["2026-03-01", 30],
        ["2026-03-02", 40],
      ]),
      series([{ key: "repository", value: "small" }], [
        ["2026-03-01", 1],
        ["2026-03-02", 2],
      ]),
      series([{ key: "repository", value: "mid" }], [
        ["2026-03-01", 5],
        ["2026-03-02", 6],
      ]),
      // One point cannot draw a line.
      series([{ key: "repository", value: "thin" }], [["2026-03-01", 99]]),
    ];
    const { multiples, max } = smallMultiples(input, "repository", 2);
    expect(multiples.map((m) => m.value)).toEqual(["big", "mid"]);
    expect(max).toBe(40);
  });
});

describe("dayHourMatrix", () => {
  it("folds day buckets into weekday × hour-block cells, UTC", () => {
    const input = [
      // 2026-03-02 is a Monday.
      series([{ key: "hour_block", value: "08" }], [
        ["2026-03-02", 3],
        ["2026-03-09", 2],
      ]),
      series([{ key: "hour_block", value: "22" }], [["2026-03-07", 4]]),
    ];
    const { cells, max, total } = dayHourMatrix(input);
    expect(cells[0][4]).toBe(5); // Monday, block "08".
    expect(cells[5][11]).toBe(4); // Saturday, block "22".
    expect(max).toBe(5);
    expect(total).toBe(9);
  });

  it("ignores series without a recognizable hour block", () => {
    const { total } = dayHourMatrix([
      series([{ key: "hour_block", value: "3" }], [["2026-03-02", 5]]),
    ]);
    expect(total).toBe(0);
  });
});

describe("hourColumns", () => {
  it("orders blocks and computes the unweighted band", () => {
    const rows: DimRow[] = [
      { value: 90, dimensions: [{ key: "hour_block", value: "10", label: "10–12" }] },
      { value: 80, dimensions: [{ key: "hour_block", value: "00", label: "00–02" }] },
      { value: 70, dimensions: [{ key: "hour_block", value: "22", label: "22–24" }] },
    ];
    const { columns, mean: m, stddev } = hourColumns(rows);
    expect(columns.map((c) => c.block)).toEqual(["00", "10", "22"]);
    expect(m).toBeCloseTo(80);
    expect(stddev).toBeCloseTo(10);
  });
});

describe("splitDateRange", () => {
  it("gives the odd day to the second half", () => {
    expect(splitDateRange({ from: "2026-03-01", to: "2026-03-07" })).toEqual({
      first: { from: "2026-03-01", to: "2026-03-03" },
      second: { from: "2026-03-04", to: "2026-03-07" },
    });
  });

  it("refuses windows that cannot yield two-day halves", () => {
    expect(splitDateRange({ from: "2026-03-01", to: "2026-03-03" })).toBeNull();
    expect(splitDateRange({ from: "2026-03-05", to: "2026-03-01" })).toBeNull();
  });
});

describe("halvesComparison", () => {
  it("keeps only values present in both halves, biggest mover first", () => {
    const first: DimRow[] = [
      { value: 80, dimensions: [{ key: "repository", value: "a", label: "A" }] },
      { value: 90, dimensions: [{ key: "repository", value: "b" }] },
      { value: 50, dimensions: [{ key: "repository", value: "gone" }] },
    ];
    const second: DimRow[] = [
      { value: 95, dimensions: [{ key: "repository", value: "a", label: "A" }] },
      { value: 88, dimensions: [{ key: "repository", value: "b" }] },
      { value: 10, dimensions: [{ key: "repository", value: "new" }] },
    ];
    const rows = halvesComparison(first, second, "repository");
    expect(rows.map((r) => [r.value, r.delta])).toEqual([
      ["a", 15],
      ["b", -2],
    ]);
    expect(rows[0].label).toBe("A");
  });
});

describe("dumbbellPairs", () => {
  it("pairs both splits per value, widest left-over-right gap first", () => {
    const rows: DimRow[] = [
      runRow("slowfail", "push", "failure", 30),
      runRow("slowfail", "push", "success", 10),
      runRow("fastfail", "push", "failure", 2),
      runRow("fastfail", "push", "success", 12),
      // Only one side observed — no pair, no row.
      runRow("halfpipe", "push", "success", 7),
    ];
    const pairs = dumbbellPairs(rows, "pipeline", "outcome", "failure", "success");
    expect(pairs.map((p) => p.value)).toEqual(["slowfail", "fastfail"]);
    expect(pairs[0]).toMatchObject({ left: 30, right: 10 });
  });
});

describe("scatterPoints", () => {
  it("joins axes by dimension value and rules medians", () => {
    const x: DimRow[] = [
      { value: 10, dimensions: [{ key: "repository", value: "a", label: "A" }] },
      { value: 20, dimensions: [{ key: "repository", value: "b" }] },
      { value: 30, dimensions: [{ key: "repository", value: "c" }] },
    ];
    const y: DimRow[] = [
      { value: 90, dimensions: [{ key: "repository", value: "a" }] },
      { value: 70, dimensions: [{ key: "repository", value: "b" }] },
      // "c" has no y — not a point.
    ];
    const size: DimRow[] = [
      { value: 5, dimensions: [{ key: "repository", value: "a" }] },
    ];
    const { points, medianX, medianY } = scatterPoints(x, y, size, "repository");
    expect(points).toEqual([
      { value: "a", label: "A", x: 10, y: 90, size: 5 },
      { value: "b", label: "b", x: 20, y: 70, size: undefined },
    ]);
    expect(medianX).toBe(15);
    expect(medianY).toBe(80);
  });
});

describe("calloutPair", () => {
  it("pairs the headline with the unweighted mean over groups", () => {
    const rows: DimRow[] = [
      { value: 90, dimensions: [{ key: "repository", value: "a" }] },
      { value: 50, dimensions: [{ key: "repository", value: "b" }] },
    ];
    expect(calloutPair(88, rows, "repository")).toEqual({
      headline: 88,
      unweightedMean: 70,
      groups: 2,
    });
  });

  it("needs a headline and at least two groups", () => {
    const one: DimRow[] = [
      { value: 90, dimensions: [{ key: "repository", value: "a" }] },
    ];
    expect(calloutPair(88, one, "repository")).toBeNull();
    expect(calloutPair(null, one, "repository")).toBeNull();
  });
});

describe("weeklyVerdicts", () => {
  const weeks = (values: number[]): Array<[string, number]> =>
    values.map((value, i) => [`2026-0${i + 1}-01`, value]);

  it("resolves mean and spread to the documented ladder", () => {
    const input = [
      series([{ key: "pipeline", value: "solid" }], weeks([97, 96, 98, 97, 96])),
      series([{ key: "pipeline", value: "erratic" }], weeks([95, 40, 90, 30, 85])),
      series([{ key: "pipeline", value: "struggling" }], weeks([60, 62, 61, 60, 62])),
      series([{ key: "pipeline", value: "healthy" }], weeks([88, 90, 86, 89, 87])),
    ];
    const { verdicts, thin } = weeklyVerdicts(input, "pipeline", 5);
    expect(thin).toBe(0);
    expect(
      Object.fromEntries(verdicts.map((v) => [v.value, v.verdict]))
    ).toEqual({
      solid: "solid",
      erratic: "erratic",
      struggling: "struggling",
      healthy: "healthy",
    });
    // Worst mean first — the reader starts at the problem.
    expect(verdicts[0].value).toBe("struggling");
  });

  it("leaves thin histories unjudged and counts them", () => {
    const { verdicts, thin } = weeklyVerdicts(
      [series([{ key: "pipeline", value: "new" }], weeks([90, 91]))],
      "pipeline",
      5
    );
    expect(verdicts).toEqual([]);
    expect(thin).toBe(1);
  });
});
