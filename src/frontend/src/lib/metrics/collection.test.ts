import { describe, expect, it } from "vitest";

import {
  MEDIAN_METRIC_FIXTURE,
  RATIO_METRIC_FIXTURE,
  SUM_METRIC_FIXTURE,
} from "@/mocks/metric-results-fixtures";
import {
  buildMetricCollectionRequest,
  chunkEntityIds,
  entityChunkSize,
  forEntity,
  mergeNormalizedResults,
  normalizeMetricResult,
  normalizeMetricResults,
  projectViews,
  resolveBucket,
  type MetricCollectionConfig,
} from "@/lib/metrics/collection";

const COLLECTION: MetricCollectionConfig = {
  metrics: [
    {
      key: "ai.accepted_lines",
      views: [
        { view: "period" },
        { view: "peer" },
        {
          view: "timeseries",
          bucket: "auto",
          dimensions: ["tool"],
          groupLimit: {
            count: 10,
            rank_by_metric: "ai.accepted_lines",
            include_remainder: true,
          },
        },
        { view: "breakdown", dimensions: ["tool"] },
      ],
    },
    {
      key: "ai.tool_acceptance_rate",
      views: [{ view: "period" }, { view: "peer" }],
    },
  ],
};

const RANGE = { from: "2026-06-01", to: "2026-06-30" };

describe("buildMetricCollectionRequest", () => {
  it("derives the wire request from the collection", () => {
    const request = buildMetricCollectionRequest(
      COLLECTION,
      { type: "person", ids: ["alice@example.com"] },
      RANGE
    );
    expect(request.entity).toEqual({
      type: "person",
      ids: ["alice@example.com"],
    });
    expect(request.metrics).toHaveLength(2);
    expect(request.metrics[0]?.views).toEqual([
      { view: "period" },
      { view: "peer" },
      {
        view: "timeseries",
        bucket: "week",
        dimensions: ["tool"],
        group_limit: {
          count: 10,
          rank_by_metric: "ai.accepted_lines",
          include_remainder: true,
        },
      },
      { view: "breakdown", dimensions: ["tool"] },
    ]);
  });
});

describe("projectViews", () => {
  it("derives a view subset without touching the source collection", () => {
    const projected = projectViews(COLLECTION, ["period", "peer"]);
    expect(
      projected.metrics.every((m) =>
        m.views.every((v) => v.view === "period" || v.view === "peer")
      )
    ).toBe(true);
    expect(COLLECTION.metrics[0]?.views).toHaveLength(4);
  });

  it("drops metrics whose views project to empty (backend rejects views: [])", () => {
    const chartOnly: MetricCollectionConfig = {
      metrics: [
        {
          key: "m.chart",
          views: [{ view: "timeseries", bucket: "auto" }],
        },
        { key: "m.full", views: [{ view: "period" }, { view: "peer" }] },
      ],
    };
    const projected = projectViews(chartOnly, ["period", "peer"]);
    expect(projected.metrics.map((m) => m.key)).toEqual(["m.full"]);
  });
});

describe("normalizeMetricResult forward-compat", () => {
  it("ignores unknown view kinds instead of crashing the collection", () => {
    const withUnknown = {
      ...SUM_METRIC_FIXTURE,
      views: [
        ...SUM_METRIC_FIXTURE.views,
        { view: "sparkline", points: [] } as never,
      ],
    };
    const normalized = normalizeMetricResult(withUnknown);
    expect(normalized.period).toBeDefined();
    expect(normalized.peer).toBeDefined();
  });
});

describe("row-limit chunking", () => {
  const PERIOD_PEER: MetricCollectionConfig = {
    metrics: Array.from({ length: 10 }, (_, i) => ({
      key: `m.${i}`,
      views: [{ view: "period" as const }, { view: "peer" as const }],
    })),
  };

  it("computes the per-request entity capacity from the view count", () => {
    // 10 metrics × 2 views = 20 rows/entity; 4500 / 20 = 225.
    expect(entityChunkSize(PERIOD_PEER)).toBe(225);
  });

  it("is not chunkable with bucket-projected views", () => {
    expect(entityChunkSize(COLLECTION)).toBeNull();
  });

  it("splits a 251-person roster into limit-safe chunks", () => {
    const ids = Array.from({ length: 251 }, (_, i) => `p${i}@x.com`);
    const chunks = chunkEntityIds(ids, entityChunkSize(PERIOD_PEER)!);
    expect(chunks).toHaveLength(2);
    expect(chunks.flat()).toHaveLength(251);
    for (const chunk of chunks) {
      expect(chunk.length * 20).toBeLessThanOrEqual(4500);
    }
  });

  it("merges chunked results back into one collection", () => {
    const a = normalizeMetricResults([SUM_METRIC_FIXTURE]);
    const bFixture = structuredClone(SUM_METRIC_FIXTURE);
    const periodView = bFixture.views[0];
    if (periodView?.view === "period") {
      periodView.values = [{ entity_id: "carol@example.com", value: 7 }];
    }
    const merged = mergeNormalizedResults([
      a,
      normalizeMetricResults([bFixture]),
    ]);
    const metric = merged.get("ai.accepted_lines")!;
    expect(metric.period?.values.map((v) => v.entity_id)).toEqual([
      "alice@example.com",
      "bob@example.com",
      "carol@example.com",
    ]);
    // Source maps untouched.
    expect(a.get("ai.accepted_lines")?.period?.values).toHaveLength(2);
  });

  it("adopts a view from a later chunk when the accumulator lacks it", () => {
    // First chunk: period only. Second chunk: peer only. The merged result
    // must carry both (one-sided branches, not just both-present).
    const periodOnly = structuredClone(SUM_METRIC_FIXTURE);
    periodOnly.views = periodOnly.views.filter((v) => v.view === "period");
    const peerOnly = structuredClone(SUM_METRIC_FIXTURE);
    peerOnly.views = peerOnly.views.filter((v) => v.view === "peer");

    const merged = mergeNormalizedResults([
      normalizeMetricResults([periodOnly]),
      normalizeMetricResults([peerOnly]),
    ]);
    const metric = merged.get("ai.accepted_lines")!;
    expect(metric.period).toBeDefined();
    expect(metric.peer).toBeDefined();
  });

  it("keeps the accumulator's view when a later chunk omits it", () => {
    const full = normalizeMetricResults([SUM_METRIC_FIXTURE]);
    const peerOnly = structuredClone(SUM_METRIC_FIXTURE);
    peerOnly.views = peerOnly.views.filter((v) => v.view === "peer");

    const merged = mergeNormalizedResults([
      full,
      normalizeMetricResults([peerOnly]),
    ]);
    const metric = merged.get("ai.accepted_lines")!;
    // period survives (second chunk had none); peer values append.
    expect(metric.period?.values.length).toBeGreaterThan(0);
    expect(metric.peer?.values.length ?? 0).toBeGreaterThan(1);
  });
});

describe("histogram view", () => {
  const HISTOGRAM_COLLECTION: MetricCollectionConfig = {
    metrics: [
      {
        key: "git.pr_cycle_time_h",
        views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
      },
    ],
  };

  it("derives the histogram wire view", () => {
    const request = buildMetricCollectionRequest(
      HISTOGRAM_COLLECTION,
      { type: "person", ids: ["alice@example.com"] },
      RANGE
    );
    expect(request.metrics[0]?.views).toEqual([
      { view: "period" },
      { view: "peer" },
      { view: "histogram" },
    ]);
  });

  it("normalizes onto the histogram field", () => {
    const normalized = normalizeMetricResult(MEDIAN_METRIC_FIXTURE);
    expect(normalized.computation).toBe("median");
    expect(normalized.histogram?.values).toHaveLength(1);
  });

  it("slices bins for the entity and returns none for others", () => {
    const metric = normalizeMetricResults([MEDIAN_METRIC_FIXTURE]).get(
      "git.pr_cycle_time_h"
    )!;
    expect(forEntity(metric, "alice@example.com").histogram).toHaveLength(1);
    expect(
      forEntity(metric, "alice@example.com").histogram[0]?.bins.length
    ).toBeGreaterThan(0);
    expect(forEntity(metric, "bob@example.com").histogram).toHaveLength(0);
  });

  it("is not chunkable (single-entity drilldown view)", () => {
    expect(entityChunkSize(HISTOGRAM_COLLECTION)).toBeNull();
  });
});

describe("resolveBucket", () => {
  it("keeps explicit buckets", () => {
    expect(resolveBucket("month", RANGE)).toBe("month");
  });

  it("tiers auto: day through 7d, week below 1y, month from 1y", () => {
    expect(
      resolveBucket("auto", { from: "2026-06-01", to: "2026-06-07" })
    ).toBe("day");
    expect(
      resolveBucket("auto", { from: "2026-06-01", to: "2026-06-08" })
    ).toBe("week");
    expect(
      resolveBucket("auto", { from: "2025-07-22", to: "2026-07-20" })
    ).toBe("week");
    expect(
      resolveBucket("auto", { from: "2025-07-21", to: "2026-07-20" })
    ).toBe("month");
  });
});

describe("normalizeMetricResult", () => {
  it("maps views onto per-view fields and keeps wire metadata", () => {
    const normalized = normalizeMetricResult(SUM_METRIC_FIXTURE);
    expect(normalized.computation).toBe("sum");
    expect(normalized.explanation).toBeDefined();
    expect(normalized.period).toBeDefined();
    expect(normalized.peer).toBeDefined();
    expect(normalized.timeseries).toBeDefined();
    expect(normalized.breakdown).toBeDefined();
    expect("views" in normalized).toBe(false);
  });

  it("carries ratio scale and null unit", () => {
    const normalized = normalizeMetricResult(RATIO_METRIC_FIXTURE);
    expect(normalized.scale).toBe(100);
    expect(normalized.unit).toBeNull();
  });
});

describe("forEntity", () => {
  const byKey = normalizeMetricResults([SUM_METRIC_FIXTURE]);
  const metric = byKey.get("ai.accepted_lines")!;

  it("slices every view for one entity", () => {
    const data = forEntity(metric, "alice@example.com");
    expect(data.value).toBe(1240);
    expect(data.peer?.n).toBe(12);
    expect(data.bucket).toBe("day");
    expect(data.series).toHaveLength(2);
    expect(data.breakdown).toHaveLength(2);
  });

  it("returns empty slices for unknown entities", () => {
    const data = forEntity(metric, "nobody@example.com");
    expect(data.value).toBeNull();
    expect(data.peer).toBeNull();
    expect(data.series).toHaveLength(0);
    expect(data.breakdown).toHaveLength(0);
  });

  it("zero-fills sums for requested-but-inactive entities (backend contract)", () => {
    expect(forEntity(metric, "bob@example.com").value).toBe(0);
  });

  it("keeps a null period value distinct from a missing entity", () => {
    const ratio = normalizeMetricResults([RATIO_METRIC_FIXTURE]).get(
      "ai.tool_acceptance_rate"
    )!;
    expect(forEntity(ratio, "bob@example.com").value).toBeNull();
    expect(forEntity(ratio, "nobody@example.com").value).toBeNull();
  });
});
