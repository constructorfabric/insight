import { describe, expect, it } from "vitest";

import {
  buildPeerStoryEntries,
  partitionPeerStory,
} from "@/lib/metrics/peer-story";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import type { MetricCollectionConfig } from "@/lib/metrics/collection";
import type { MetricResult } from "@/api/metric-results-client";

function metric(
  key: string,
  value: number | null,
  opts: { direction?: MetricResult["direction"]; median?: number } = {},
): MetricResult {
  const median = opts.median ?? 10;
  return {
    metric_key: key,
    label: key,
    unit: null,
    format: "integer",
    direction: opts.direction ?? "higher_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "me@x.com", value }] },
      {
        view: "peer",
        values: [
          {
            entity_id: "me@x.com",
            target_value: value,
            p25: median * 0.5,
            median,
            p75: median * 1.5,
            min: 0,
            max: median * 3,
            n: 10,
          },
        ],
      },
    ],
  };
}

const COLLECTION: MetricCollectionConfig = {
  metrics: ["m.win", "m.issue", "m.par", "m.absent"].map((key) => ({
    key,
    views: [{ view: "period" }, { view: "peer" }],
  })),
};

describe("buildPeerStoryEntries", () => {
  it("builds entries in collection order, dropping valueless metrics", () => {
    const byKey = normalizeMetricResults([
      metric("m.win", 30),
      metric("m.issue", 1),
      metric("m.par", 10),
      metric("m.absent", null),
    ]);
    const entries = buildPeerStoryEntries(COLLECTION, byKey, "me@x.com");
    expect(entries.map((e) => e.key)).toEqual(["m.win", "m.issue", "m.par"]);
    expect(entries[0]?.status).toBe("top");
    expect(entries[1]?.status).toBe("bottom");
    expect(entries[2]?.status).toBe("in_pack");
  });

  it("keeps unmeasured people neutral instead of bottom-branding them", () => {
    // Period views zero-fill the own total, but a null peer target_value
    // means the person has no observations — no standing, not "bottom 25%".
    const unmeasured = metric("m.win", 0);
    const peerView = unmeasured.views[1];
    if (peerView?.view === "peer" && peerView.values[0]) {
      peerView.values[0].target_value = null;
    }
    const byKey = normalizeMetricResults([unmeasured]);
    const collection: MetricCollectionConfig = {
      metrics: [{ key: "m.win", views: [{ view: "period" }, { view: "peer" }] }],
    };
    const entries = buildPeerStoryEntries(collection, byKey, "me@x.com");
    expect(entries[0]?.status).toBe("neutral");
    expect(entries[0]?.severity).toBe(0);
  });

  it("ranks lower-is-better below-median as top while the gap stays arithmetic", () => {
    const byKey = normalizeMetricResults([
      metric("m.win", 2, { direction: "lower_is_better" }),
    ]);
    const collection: MetricCollectionConfig = {
      metrics: [{ key: "m.win", views: [{ view: "period" }, { view: "peer" }] }],
    };
    const entries = buildPeerStoryEntries(collection, byKey, "me@x.com");
    expect(entries[0]?.status).toBe("top");
    expect(entries[0]?.gapDelta).toBe(-8);
  });
});

describe("partitionPeerStory", () => {
  const byKey = normalizeMetricResults([
    metric("m.win", 30),
    metric("m.issue", 1),
    metric("m.par", 10),
  ]);
  const entries = buildPeerStoryEntries(COLLECTION, byKey, "me@x.com");

  it("puts the most severe outlier in the hero slot, bottom first", () => {
    const { hero, folded } = partitionPeerStory(entries, "all");
    expect(hero?.key).toBe("m.issue");
    expect(folded.map((e) => e.key)).toEqual(["m.par"]);
  });

  it("focus critical shows only bottom outliers", () => {
    const { focusedOutliers } = partitionPeerStory(entries, "critical");
    expect(focusedOutliers.map((e) => e.key)).toEqual(["m.issue"]);
  });

  it("focus rewards shows only top outliers", () => {
    const { focusedOutliers, hero } = partitionPeerStory(entries, "rewards");
    expect(focusedOutliers.map((e) => e.key)).toEqual(["m.win"]);
    expect(hero?.key).toBe("m.win");
  });

  it("neutral focus surfaces no outliers", () => {
    const { hero, focusedOutliers } = partitionPeerStory(entries, "neutral");
    expect(hero).toBeNull();
    expect(focusedOutliers).toHaveLength(0);
  });
});

// When the cohort median is 0 the percentage gap is undefined, so severity
// falls back to gap / peerSpread. This exercises the IQR and min–max range
// branches plus the flat-pool gate (a zero-spread cohort ranks nobody, so it
// carries no severity at all).
describe("peerSpread fallback (median 0)", () => {
  function metricWithStats(
    key: string,
    stats: { p25: number; p75: number; min: number; max: number },
  ): MetricResult {
    return {
      metric_key: key,
      label: key,
      unit: null,
      format: "integer",
      direction: "higher_is_better",
      computation: "sum",
      views: [
        { view: "period", values: [{ entity_id: "me@x.com", value: 5 }] },
        {
          view: "peer",
          values: [
            { entity_id: "me@x.com", target_value: 5, median: 0, n: 10, ...stats },
          ],
        },
      ],
    };
  }

  const collection: MetricCollectionConfig = {
    metrics: ["iqr", "range", "constant"].map((key) => ({
      key,
      views: [{ view: "period" }, { view: "peer" }],
    })),
  };

  it("scales severity by the widest available spread", () => {
    const byKey = normalizeMetricResults([
      metricWithStats("iqr", { p25: -5, p75: 5, min: -8, max: 8 }), // spread 10
      metricWithStats("range", { p25: 0, p75: 0, min: -10, max: 10 }), // spread 20
      metricWithStats("constant", { p25: 0, p75: 0, min: 0, max: 0 }), // flat
    ]);
    const byKeyMap = new Map(
      buildPeerStoryEntries(collection, byKey, "me@x.com").map((e) => [
        e.key,
        e.severity,
      ]),
    );
    // gap is 5 for both spread pools; severity = 5 / spread. A flat pool
    // (zero spread) is ineligible for ranking — severity 0, never an outlier.
    expect(byKeyMap.get("iqr")).toBeCloseTo(0.5);
    expect(byKeyMap.get("range")).toBeCloseTo(0.25);
    expect(byKeyMap.get("constant")).toBe(0);
  });
});
