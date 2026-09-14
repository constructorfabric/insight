/**
 * The mock has to reproduce the SHAPE the real endpoint returns, because that
 * shape is what the lens is built against. The reference numbers in the
 * assertions come from a real read of the gold ops view (see the factory's
 * header): 30d @ 15m spans 1 … ~6800 rows per bucket, a 24h window holds ~97
 * buckets, and a 30m @ 1s window holds ~1800.
 */
import { describe, expect, it } from "vitest";

import { buildIngestionIntensity } from "./ingestion-factory";

const NOW = Date.parse("2026-08-27T09:15:00.000Z");
const DAY = 86_400_000;

describe("the default windows match the server's", () => {
  it("gives a day of 15-minute buckets", () => {
    const body = buildIngestionIntensity({ grain: "15m", now: NOW });
    const buckets = new Set(body.points.map((p) => p.bucket));
    // 96 slots in 24h; idle buckets are omitted, so allow for the gaps.
    expect(buckets.size).toBeGreaterThan(80);
    expect(buckets.size).toBeLessThanOrEqual(97);
    expect(body.grain).toBe("15m");
    expect(body.series).toBe("connector");
  });

  it("gives thirty minutes of one-second buckets", () => {
    const body = buildIngestionIntensity({ grain: "1s", now: NOW });
    const buckets = new Set(body.points.map((p) => p.bucket));
    expect(buckets.size).toBeGreaterThan(1_600);
    expect(buckets.size).toBeLessThanOrEqual(1_800);
  });
});

describe("the shape the charts depend on", () => {
  const trend = buildIngestionIntensity({
    grain: "15m",
    series: "total",
    from: new Date(NOW - 30 * DAY).toISOString(),
    to: new Date(NOW).toISOString(),
    now: NOW,
  });

  it("spans orders of magnitude, which is why the axis is logarithmic", () => {
    const rows = trend.points.map((p) => p.rows);
    expect(Math.min(...rows)).toBe(1);
    expect(Math.max(...rows)).toBeGreaterThan(1_000);
  });

  it("includes buckets holding exactly one row", () => {
    // The case a log axis floored at exactly 1 renders as nothing.
    expect(trend.points.filter((p) => p.rows === 1).length).toBeGreaterThan(0);
  });

  it("omits idle buckets instead of sending zeros", () => {
    // An absent bucket is a gap on a numeric time axis; a zero would draw a
    // bar of no height and stack an invisible segment.
    const slots = Math.floor((30 * DAY) / (15 * 60 * 1_000));
    const buckets = new Set(trend.points.map((p) => p.bucket));
    expect(buckets.size).toBeLessThan(slots);
    expect(trend.points.every((p) => p.rows > 0)).toBe(true);
  });

  it("is deterministic, so a re-render draws the same chart", () => {
    const again = buildIngestionIntensity({
      grain: "15m",
      series: "total",
      from: new Date(NOW - 30 * DAY).toISOString(),
      to: new Date(NOW).toISOString(),
      now: NOW,
    });
    expect(again.points).toEqual(trend.points);
  });

  it("bands a total read under the single `all` key", () => {
    expect(new Set(trend.points.map((p) => p.key))).toEqual(new Set(["all"]));
  });
});

describe("bucket stamps match the server's formats", () => {
  it("is zone-less, and only the 1s grain carries milliseconds", () => {
    const wide = buildIngestionIntensity({ grain: "15m", now: NOW });
    const live = buildIngestionIntensity({ grain: "1s", now: NOW });
    expect(wide.points[0].bucket).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
    expect(live.points[0].bucket).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}$/);
  });

  it("aligns buckets to the grain", () => {
    const wide = buildIngestionIntensity({ grain: "15m", now: NOW });
    for (const point of wide.points.slice(0, 20)) {
      expect(Number(point.bucket.slice(14, 16)) % 15).toBe(0);
    }
  });
});

describe("scoping", () => {
  it("bands by the streams of the scoped connector", () => {
    const body = buildIngestionIntensity({
      grain: "15m",
      scope: "bronze_demo_tasks",
      now: NOW,
    });
    expect(body.scope).toBe("bronze_demo_tasks");
    expect(body.series).toBe("stream");
    expect(new Set(body.points.map((p) => p.key))).toEqual(
      new Set(["issues", "comments", "boards"]),
    );
  });

  it("bands by connector when unscoped", () => {
    const body = buildIngestionIntensity({ grain: "15m", now: NOW });
    expect(body.scope).toBeUndefined();
    expect(new Set(body.points.map((p) => p.key))).toEqual(
      new Set(["demo_tasks", "demo_chat", "demo_docs"]),
    );
  });
});
