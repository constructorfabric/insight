import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  buildActiveContributorData,
  buildTrendData,
  pickTrendBucket,
} from "./trend-data";

const RANGE_30D = { from: "2026-06-24", to: "2026-07-23" };

describe("pickTrendBucket", () => {
  const YEAR = { from: "2025-07-24", to: "2026-07-23" };

  it("keeps daily buckets for a small team", () =>
    expect(pickTrendBucket(5, RANGE_30D)).toBe("day"));

  it("coarsens to weekly once daily rows outgrow the limit", () =>
    expect(pickTrendBucket(200, RANGE_30D)).toBe("week"));

  it("charts a year monthly for a roster the backend can still answer", () => {
    // The budget is per metric, so a three-series trend does not divide it.
    // This roster used to be refused outright at a year.
    expect(pickTrendBucket(300, YEAR)).toBe("month");
  });

  it("gives up when the roster alone exceeds one bucket", () => {
    // 4000 rows per bucket against a 4500 limit: nothing to answer with.
    expect(pickTrendBucket(4000, { from: "2026-01-01", to: "2026-01-07" })).toBeNull();
  });

  it("gives up when even monthly does not fit", () =>
    expect(pickTrendBucket(4000, YEAR)).toBeNull());

  it("counts the months a window touches, not the months it is long", () => {
    // 365 days spanning 13 calendar months; days / 30.44 says 12. A roster
    // that fits 12 buckets but not 13 must be refused, not charted against a
    // projection one bucket short of what the response carries.
    const tooWideForThirteen = Math.floor(4500 / 13) - 1;

    expect(pickTrendBucket(tooWideForThirteen, YEAR)).toBeNull();
  });

  it("never claims a bucket whose projection exceeds the limit", () => {
    // The property behind all four outcomes, checked against the backend's own
    // row count — one row per (member, bucket) plus a total row per member.
    const bucketDays = { day: 1, week: 7, month: 28 };
    for (const members of [1, 10, 152, 900, 4000]) {
      for (const days of [7, 30, 180, 365, 1000]) {
        const range = {
          from: "2026-01-01",
          to: new Date(Date.UTC(2026, 0, days)).toISOString().slice(0, 10),
        };
        const bucket = pickTrendBucket(members, range);
        if (bucket === null) continue;
        const buckets = Math.floor(days / bucketDays[bucket]) + 1;
        expect(
          members * (buckets + 1),
          `${members}p/${days}d → ${bucket}`,
        ).toBeLessThanOrEqual(4500);
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

describe("buildActiveContributorData", () => {
  const result = {
    metric_key: "git.prs_merged",
    computation: "sum",
    timeseries: {
      view: "timeseries",
      bucket: "week",
      series: [
        {
          entity_id: "a",
          points: [
            { bucket_start: "2026-06-29", value: 2 },
            { bucket_start: "2026-07-06", value: 0 },
          ],
        },
        {
          entity_id: "b",
          points: [
            { bucket_start: "2026-06-29", value: 1 },
            { bucket_start: "2026-07-06", value: 4 },
          ],
        },
      ],
    },
  } as unknown as NormalizedMetricResult;

  const byKey = new Map([["git.prs_merged", result]]);

  it("counts the people who contributed in each bucket", () => {
    const data = buildActiveContributorData("git.prs_merged", byKey, ["a", "b"]);

    expect(data).toEqual([
      { date: "2026-06-29", active: 2 },
      { date: "2026-07-06", active: 1 },
    ]);
  });

  it("counts a person once however many readings they have", () => {
    const twice = {
      ...result,
      timeseries: {
        ...result.timeseries,
        series: [
          {
            entity_id: "a",
            points: [
              { bucket_start: "2026-06-29", value: 1 },
              { bucket_start: "2026-06-29", value: 3 },
            ],
          },
        ],
      },
    } as unknown as NormalizedMetricResult;

    const data = buildActiveContributorData(
      "git.prs_merged",
      new Map([["git.prs_merged", twice]]),
      ["a"]
    );

    expect(data).toEqual([{ date: "2026-06-29", active: 1 }]);
  });

  it("leaves out people outside the roster", () => {
    const data = buildActiveContributorData("git.prs_merged", byKey, ["a"]);

    expect(data).toEqual([
      { date: "2026-06-29", active: 1 },
      { date: "2026-07-06", active: 0 },
    ]);
  });

  it("is empty when the metric was never fetched", () => {
    expect(buildActiveContributorData("missing", byKey, ["a"])).toEqual([]);
  });
});
