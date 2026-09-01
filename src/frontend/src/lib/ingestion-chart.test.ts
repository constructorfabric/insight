/**
 * The two things this file exists to pin are the two that were got wrong
 * before: buckets must be read as UTC regardless of where the reader sits, and
 * a log axis must leave room under the smallest possible bar.
 */
import { describe, expect, it } from "vitest";

import type { IngestionPoint } from "@/api/ingestion-client";
import {
  LOG_FLOOR,
  bandLabel,
  bucketMs,
  bucketToEpoch,
  connectorLabel,
  formatUtcBucket,
  formatUtcDay,
  logTicks,
  lookbackFrom,
  paddedDomain,
  pivotIntensity,
  scopeForConnector,
  seriesColorVar,
  totalsByKey,
} from "@/lib/ingestion-chart";

function point(bucket: string, key: string, rows: number): IngestionPoint {
  return { bucket, key, rows };
}

describe("bucket timestamps are UTC", () => {
  it("reads a zone-less server bucket as UTC, not as local time", () => {
    // The server emits `YYYY-MM-DD HH:MM:SS` with no marker. `Date.parse` of
    // that shape is LOCAL, which would slide every bar by the reader's offset.
    expect(bucketToEpoch("2026-08-26 14:30:00")).toBe(
      Date.parse("2026-08-26T14:30:00Z"),
    );
  });

  it("reads the millisecond form the 1s grain emits", () => {
    // toStartOfSecond() over a DateTime64(3) stringifies WITH milliseconds,
    // while toStartOfInterval() at 15 minutes does not. Both reach this parser.
    expect(bucketToEpoch("2026-08-26 14:30:45.000")).toBe(
      Date.parse("2026-08-26T14:30:45Z"),
    );
    expect(bucketToEpoch("2026-08-26 14:30:45.250")).toBe(
      Date.parse("2026-08-26T14:30:45.250Z"),
    );
  });

  it("labels buckets from UTC fields only", () => {
    const epoch = Date.parse("2026-08-26T14:30:45Z");
    expect(formatUtcBucket(epoch, "15m")).toBe("26 Aug 14:30");
    expect(formatUtcBucket(epoch, "1s")).toBe("14:30:45");
    expect(formatUtcDay(epoch)).toBe("26 Aug");
  });

  it("labels an instant that falls on a different local day", () => {
    // Late-UTC instants are the previous or next day in many zones; the label
    // must name the UTC one either way.
    expect(formatUtcBucket(Date.parse("2026-08-26T23:45:00Z"), "15m")).toBe(
      "26 Aug 23:45",
    );
    expect(formatUtcBucket(Date.parse("2026-08-27T00:15:00Z"), "15m")).toBe(
      "27 Aug 00:15",
    );
  });
});

describe("the log axis leaves room for a single row", () => {
  it("floors below one, so a one-row bucket has height", () => {
    // A bar is drawn from the axis minimum: a floor of exactly 1 gives every
    // one-row bucket zero height and the sparse tail disappears.
    expect(LOG_FLOOR).toBeLessThan(1);
    expect(LOG_FLOOR).toBeGreaterThan(0);
  });

  it("ticks whole powers of ten spanning the data", () => {
    expect(logTicks(1)).toEqual([1]);
    expect(logTicks(9)).toEqual([1, 10]);
    expect(logTicks(50_000)).toEqual([1, 10, 100, 1000, 10_000, 100_000]);
  });

  it("still ticks when nothing was extracted", () => {
    expect(logTicks(0)).toEqual([1]);
  });
});

describe("colour follows the entity", () => {
  it("is a pure function of the name", () => {
    expect(seriesColorVar("jira")).toBe(seriesColorVar("jira"));
  });

  it("does not move when the neighbouring set changes", () => {
    // The whole point: a connector keeps its swatch when the window changes,
    // when a neighbour stops ingesting, and across the two pages.
    const alone = seriesColorVar("bamboohr");
    const crowded = totalsByKey([
      point("2026-08-26 00:00:00", "bamboohr", 1),
      point("2026-08-26 00:00:00", "slack", 9),
    ]).map((row) => seriesColorVar(row.key));
    expect(crowded).toContain(alone);
  });

  it("resolves to a palette slot that exists", () => {
    // index.css defines --chart-1 … --chart-12 and repeats them in the
    // dark-mode block. An index past 12 is an undefined var(), which paints the
    // band the fallback colour instead of failing loudly — so the ceiling is
    // asserted over enough keys to catch a wrong PALETTE_SIZE.
    const keys = [
      "jira", "slack", "zoom", "_boards", "m365", "cursor", "bamboohr",
      "confluence", "bitbucket_cloud", "claude_enterprise", "demo_tasks",
      "demo_chat", "demo_docs", "issues", "comments", "boards", "messages",
      "channels", "pages", "github", "gitlab", "hubspot", "outline", "zendesk",
    ];
    for (const key of keys) {
      expect(seriesColorVar(key), key).toMatch(/^var\(--chart-(?:[1-9]|1[0-2])\)$/);
    }
  });
});

describe("pivoting flat points into stackable rows", () => {
  const points = [
    point("2026-08-26 00:00:00", "jira", 100),
    point("2026-08-26 00:00:00", "slack", 5),
    point("2026-08-26 00:30:00", "jira", 7),
  ];

  it("gives one row per bucket with a column per band", () => {
    const { rows } = pivotIntensity(points);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toMatchObject({
      epoch: Date.parse("2026-08-26T00:00:00Z"),
      jira: 100,
      slack: 5,
      total: 105,
    });
    expect(rows[1]).toMatchObject({ jira: 7, total: 7 });
  });

  it("leaves an unextracted bucket absent rather than zero-filled", () => {
    // The charts plot a numeric time axis, so a missing bucket is a gap. A
    // filled zero would also mean inventing rows for the widest window.
    const { rows } = pivotIntensity(points);
    expect(rows.map((row) => row.epoch)).toEqual([
      Date.parse("2026-08-26T00:00:00Z"),
      Date.parse("2026-08-26T00:30:00Z"),
    ]);
  });

  it("orders rows by time even when the server did not", () => {
    const { rows } = pivotIntensity([points[2], points[0]]);
    expect(rows[0].epoch).toBeLessThan(rows[1].epoch);
  });

  it("orders bands widest first so the stack matches the legend", () => {
    expect(pivotIntensity(points).keys).toEqual(["jira", "slack"]);
  });

  it("breaks a total tie by name, so the order is stable", () => {
    const tied = pivotIntensity([
      point("2026-08-26 00:00:00", "zoom", 4),
      point("2026-08-26 00:00:00", "cursor", 4),
    ]);
    expect(tied.keys).toEqual(["cursor", "zoom"]);
  });

  it("drops a bucket it cannot read rather than plotting NaN", () => {
    const { rows } = pivotIntensity([point("not a timestamp", "jira", 3)]);
    expect(rows).toEqual([]);
  });

  it("survives an empty read", () => {
    expect(pivotIntensity([])).toEqual({ rows: [], keys: [] });
  });
});

describe("roster totals", () => {
  it("sums each band across buckets, widest first", () => {
    expect(
      totalsByKey([
        point("2026-08-26 00:00:00", "jira", 10),
        point("2026-08-26 00:15:00", "jira", 5),
        point("2026-08-26 00:15:00", "slack", 20),
      ]),
    ).toEqual([
      { key: "slack", rows: 20 },
      { key: "jira", rows: 15 },
    ]);
  });
});

describe("naming", () => {
  it("talks about connectors, not bronze databases", () => {
    expect(connectorLabel("bronze_bamboohr")).toBe("bamboohr");
    expect(scopeForConnector("bamboohr")).toBe("bronze_bamboohr");
  });

  it("does not double the prefix on a value that already carries it", () => {
    expect(scopeForConnector("bronze_bamboohr")).toBe("bronze_bamboohr");
  });

  it("names the single band of a total read", () => {
    expect(bandLabel("all", "total")).toBe("All connectors");
    // `all` is only special for a total read — a connector could be called that.
    expect(bandLabel("all", "connector")).toBe("all");
    expect(bandLabel("jira", "connector")).toBe("jira");
  });
});

describe("the lookback bound", () => {
  it("anchors to the start of a UTC day, not to the instant", () => {
    // Stable for a whole day, so a chart refetching every minute asks for the
    // same window instead of sliding it.
    const midMorning = Date.parse("2026-08-26T09:41:17Z");
    const lateEvening = Date.parse("2026-08-26T23:59:59Z");
    expect(lookbackFrom(midMorning, 30)).toBe("2026-07-27T00:00:00.000Z");
    expect(lookbackFrom(lateEvening, 30)).toBe(lookbackFrom(midMorning, 30));
  });
});

describe("the plotted domain", () => {
  it("widens by half a bucket so an edge bar is not clipped", () => {
    // A bar on a numeric axis is centred on its value: without the padding the
    // first and last buckets each lose half their width off the plot, and the
    // first one overlaps the y-axis labels.
    const half = bucketMs("15m") / 2;
    expect(
      paddedDomain("2026-08-26T00:00:00Z", "2026-08-26T01:00:00Z", "15m"),
    ).toEqual([
      Date.parse("2026-08-26T00:00:00Z") - half,
      Date.parse("2026-08-26T01:00:00Z") + half,
    ]);
  });

  it("pads by the grain actually plotted", () => {
    expect(bucketMs("1s")).toBe(1_000);
    expect(bucketMs("15m")).toBe(900_000);
    const [lo] = paddedDomain("2026-08-26T00:00:00Z", "2026-08-26T00:30:00Z", "1s");
    expect(Date.parse("2026-08-26T00:00:00Z") - lo).toBe(500);
  });
});
