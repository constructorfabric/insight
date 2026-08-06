import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { buildTrendData, pickTrendBucket } from "./trend-data";

const RANGE_30D = { from: "2026-06-24", to: "2026-07-23" };

describe("pickTrendBucket", () => {
  it("keeps daily buckets for a small team", () =>
    expect(pickTrendBucket(5, 2, RANGE_30D)).toBe("day"));
  it("coarsens to weekly for the org root (152×2×30 > row limit)", () =>
    expect(pickTrendBucket(152, 2, RANGE_30D)).toBe("week"));
  it("falls to monthly for a year at org scale", () =>
    expect(pickTrendBucket(152, 2, { from: "2025-07-24", to: "2026-07-23" })).toBe("month"));

  it("gives up when the roster alone exceeds one bucket", () => {
    // 4000 × 2 = 8000 rows before any time dimension. A `Math.max(1, …)` floor
    // used to pretend one bucket always fits and answer "week" here.
    expect(pickTrendBucket(4000, 2, { from: "2026-01-01", to: "2026-01-07" })).toBeNull();
  });

  it("gives up when even monthly does not fit", () => {
    // 4000 people over a year is ~12 buckets × 4000 × 2 metrics = 96k rows
    // against a 4500 limit. The old version answered "month" and the request
    // 400'd; there is no bucket to answer with, so say so.
    expect(pickTrendBucket(4000, 2, { from: "2025-07-24", to: "2026-07-23" })).toBeNull();
  });

  it("never claims a bucket whose projection exceeds the limit", () => {
    // The property behind all four outcomes, checked across scales rather than
    // at the two thresholds a hand-picked example happens to sit on.
    const perBucketDays = { day: 1, week: 7, month: 30.44 };
    for (const members of [1, 10, 152, 900, 4000]) {
      for (const days of [7, 30, 180, 365, 1000]) {
        const range = {
          from: "2026-01-01",
          to: new Date(Date.UTC(2026, 0, days)).toISOString().slice(0, 10),
        };
        const bucket = pickTrendBucket(members, 2, range);
        if (bucket === null) continue;
        const buckets = Math.ceil(days / perBucketDays[bucket]);
        expect(members * 2 * buckets, `${members}p/${days}d → ${bucket}`).toBeLessThanOrEqual(4500);
      }
    }
  });
});

describe("buildTrendData", () => {
  it("sums per-bucket points across members and sorts by date", () => {
    const r = {
      metric_key: "m",
      computation: "sum",
      timeseries: {
        view: "timeseries",
        bucket: "week",
        series: [
          { entity_id: "a", points: [{ bucket_start: "2026-07-06", value: 2 }] },
          { entity_id: "b", points: [{ bucket_start: "2026-07-06", value: 3 }, { bucket_start: "2026-06-29", value: 1 }] },
        ],
      },
    } as unknown as NormalizedMetricResult;
    const data = buildTrendData(["m"], new Map([["m", r]]), ["a", "b"]);
    expect(data).toEqual([
      { date: "2026-06-29", m: 1 },
      { date: "2026-07-06", m: 5 },
    ]);
  });
});
