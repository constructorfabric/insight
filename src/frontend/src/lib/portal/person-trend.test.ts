/**
 * The readings a tile's line is drawn from.
 *
 * Three of the rules here exist to refuse to draw something, and each refusal
 * looks like a missing feature from the outside — which is exactly why they
 * need tests. Left untested, "why is there no line here" gets fixed by
 * deleting the rule, and the screen goes back to inventing a collapse out of
 * the calendar.
 */
import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import {
  TREND_BUCKETS,
  personTrendPoints,
  runningBucketStart,
  trendBucket,
  trendRange,
} from "./person-trend";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";

/** One metric carrying a timeseries of `[bucket_start, value]` readings. */
function withSeries(points: [string, number | null][]) {
  const result = {
    metric_key: "git.commits",
    label: "Commits",
    unit: null,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [
      {
        view: "timeseries",
        bucket: "month",
        series: [
          {
            entity_id: ME,
            dimensions: [],
            points: points.map(([bucket_start, value]) => ({
              bucket_start,
              value,
            })),
          },
        ],
      },
    ],
  } as unknown as MetricResult;
  return normalizeMetricResults([result]).get("git.commits");
}

describe("trendBucket", () => {
  it("draws the line in the unit the reader picked", () => {
    // Anything else asks them to convert in their head on every glance.
    expect(trendBucket("week")).toBe("week");
    expect(trendBucket("month")).toBe("month");
    expect(trendBucket("quarter")).toBe("month");
    expect(trendBucket("year")).toBe("month");
  });
});

describe("runningBucketStart", () => {
  it("finds the first of the month", () => {
    expect(runningBucketStart("2026-03-18", "month")).toBe("2026-03-01");
  });

  it("finds the Monday of the week, including from a Sunday", () => {
    // A Sunday is the case a naive "subtract getUTCDay()" gets wrong: it
    // would jump forward a week and drop six finished days.
    expect(runningBucketStart("2026-03-18", "week")).toBe("2026-03-16");
    expect(runningBucketStart("2026-03-22", "week")).toBe("2026-03-16");
    expect(runningBucketStart("2026-03-16", "week")).toBe("2026-03-16");
  });
});

describe("trendRange", () => {
  it("asks for the finished buckets plus the running one", () => {
    const { from, to } = trendRange("2026-03-18", "month");
    expect(to).toBe("2026-03-18");
    const days = (Date.parse(to) - Date.parse(from)) / 86_400_000;
    expect(days).toBeGreaterThan(31 * TREND_BUCKETS);
  });
});

describe("personTrendPoints", () => {
  const running = "2026-03-01";

  it("drops the running bucket", () => {
    // A month four days old holds four days of work; drawn beside finished
    // months it reads as a collapse the calendar invented.
    const points = personTrendPoints(
      withSeries([
        ["2025-12-01", 10],
        ["2026-01-01", 12],
        ["2026-02-01", 11],
        ["2026-03-01", 1],
      ]),
      ME,
      running
    );
    expect(points).toEqual([10, 12, 11]);
  });

  it("draws nothing below three readings", () => {
    // Two points always draw a straight line, and a straight line reads as a
    // steady trend — but two is also what a metric has when it has only just
    // started being measured.
    expect(
      personTrendPoints(
        withSeries([
          ["2026-01-01", 10],
          ["2026-02-01", 12],
        ]),
        ME,
        running
      )
    ).toBeNull();
  });

  it("keeps a gap as a gap rather than a zero", () => {
    // A month with no data must not become a month with none of the thing.
    expect(
      personTrendPoints(
        withSeries([
          ["2025-11-01", 10],
          ["2025-12-01", null],
          ["2026-01-01", 12],
          ["2026-02-01", 11],
        ]),
        ME,
        running
      )
    ).toEqual([10, null, 12, 11]);
  });

  it("counts only real readings towards the minimum", () => {
    // Four buckets, three of them empty: a gap must not buy its way past the
    // threshold that exists to stop two points being drawn as a trend.
    expect(
      personTrendPoints(
        withSeries([
          ["2025-11-01", 10],
          ["2025-12-01", null],
          ["2026-01-01", null],
          ["2026-02-01", 11],
        ]),
        ME,
        running
      )
    ).toBeNull();
  });

  it("keeps the most recent window, oldest first", () => {
    // More finished buckets than the line holds: the oldest fall off the
    // left, not the newest off the right.
    const extra = 4;
    const many = Array.from(
      { length: TREND_BUCKETS + extra },
      (_, i): [string, number] => [`${2020 + i}-01-01`, i]
    );
    const points = personTrendPoints(
      withSeries(many),
      ME,
      `${2020 + TREND_BUCKETS + extra}-01-01`
    );
    expect(points).toHaveLength(TREND_BUCKETS);
    expect(points?.[0]).toBe(extra);
    expect(points?.at(-1)).toBe(TREND_BUCKETS + extra - 1);
  });

  it("orders by bucket, not by arrival", () => {
    const points = personTrendPoints(
      withSeries([
        ["2026-02-01", 11],
        ["2025-12-01", 10],
        ["2026-01-01", 12],
      ]),
      ME,
      running
    );
    expect(points).toEqual([10, 12, 11]);
  });

  it("has nothing to draw for a metric that never arrived", () => {
    expect(personTrendPoints(undefined, ME, running)).toBeNull();
  });
});
