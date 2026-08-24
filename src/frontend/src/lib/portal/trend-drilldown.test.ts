import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { bucketBreakdown, mergeMemberRecords } from "./trend-drilldown";

const DATE = { key: "date", label: "Date", type: "date" as const };
const REF = { key: "ref", label: "Ref", type: "string" as const };
const TITLE = { key: "title", label: "Title", type: "string" as const };

describe("mergeMemberRecords", () => {
  it("names the person each record came from, in a leading column", () => {
    const merged = mergeMemberRecords([
      {
        personId: "a",
        name: "Ada",
        columns: [DATE, REF],
        rows: [{ values: { date: "2026-08-01", ref: "12" } }],
      },
      {
        personId: "b",
        name: "Grace",
        columns: [DATE, REF],
        rows: [{ values: { date: "2026-08-02", ref: "13" } }],
      },
    ]);

    expect(merged.columns[0]).toEqual({
      key: "who",
      label: "Who",
      type: "string",
    });
    expect(merged.rows.map((r) => r.values.who)).toEqual(["Grace", "Ada"]);
  });

  it("puts the newest record first, across people", () => {
    const merged = mergeMemberRecords([
      {
        personId: "a",
        name: "Ada",
        columns: [DATE],
        rows: [
          { values: { date: "2026-08-01" } },
          { values: { date: "2026-08-09" } },
        ],
      },
      {
        personId: "b",
        name: "Grace",
        columns: [DATE],
        rows: [{ values: { date: "2026-08-05" } }],
      },
    ]);

    expect(merged.rows.map((r) => r.values.date)).toEqual([
      "2026-08-09",
      "2026-08-05",
      "2026-08-01",
    ]);
  });

  it("unions the columns when one person's rows carry a field another's do not", () => {
    const merged = mergeMemberRecords([
      { personId: "a", name: "Ada", columns: [DATE], rows: [] },
      { personId: "b", name: "Grace", columns: [DATE, TITLE], rows: [] },
    ]);

    expect(merged.columns.map((c) => c.key)).toEqual(["who", "date", "title"]);
  });

  it("keeps the given order when the records carry no date to sort on", () => {
    const merged = mergeMemberRecords([
      {
        personId: "a",
        name: "Ada",
        columns: [REF],
        rows: [{ values: { ref: "1" } }, { values: { ref: "2" } }],
      },
    ]);

    expect(merged.rows.map((r) => r.values.ref)).toEqual(["1", "2"]);
  });
});

function result(
  points: Record<string, ReadonlyArray<[string, number | null]>>,
): NormalizedMetricResult {
  return {
    timeseries: {
      bucket: "day",
      series: Object.entries(points).map(([entity_id, pts]) => ({
        entity_id,
        points: pts.map(([bucket_start, value]) => ({ bucket_start, value })),
      })),
    },
  } as unknown as NormalizedMetricResult;
}

const MEMBERS = [
  { person_id: "a", name: "Ada" },
  { person_id: "b", name: "Grace" },
];


function multiSeriesResult(): NormalizedMetricResult {
  return {
    timeseries: {
      bucket: "day",
      series: [
        {
          entity_id: "a",
          points: [{ bucket_start: "2026-08-01", value: 2 }],
        },
        {
          entity_id: "a",
          points: [{ bucket_start: "2026-08-01", value: 3 }],
        },
      ],
    },
  } as unknown as NormalizedMetricResult;
}

describe("bucketBreakdown", () => {
  it("totals each bucket and names who contributed to it", () => {
    const byKey = new Map([
      [
        "git.prs_merged",
        result({
          a: [["2026-08-01", 2]],
          b: [["2026-08-01", 3]],
        }),
      ],
    ]);

    expect(bucketBreakdown("git.prs_merged", byKey, MEMBERS)).toEqual([
      { date: "2026-08-01", total: 5, contributors: ["Ada", "Grace"] },
    ]);
  });

  it("does not count a measured zero as a contribution", () => {
    const byKey = new Map([
      [
        "git.prs_merged",
        result({
          a: [["2026-08-01", 0]],
          b: [["2026-08-01", 4]],
        }),
      ],
    ]);

    expect(bucketBreakdown("git.prs_merged", byKey, MEMBERS)).toEqual([
      { date: "2026-08-01", total: 4, contributors: ["Grace"] },
    ]);
  });

  it("reads oldest bucket first", () => {
    const byKey = new Map([
      [
        "git.prs_merged",
        result({
          a: [
            ["2026-08-09", 1],
            ["2026-08-01", 1],
          ],
        }),
      ],
    ]);

    expect(
      bucketBreakdown("git.prs_merged", byKey, MEMBERS).map((r) => r.date),
    ).toEqual(["2026-08-01", "2026-08-09"]);
  });

  it("counts a person once in a bucket they have several readings in", () => {
    const byKey = new Map([["git.prs_merged", multiSeriesResult()]]);

    expect(bucketBreakdown("git.prs_merged", byKey, MEMBERS)).toEqual([
      { date: "2026-08-01", total: 5, contributors: ["Ada"] },
    ]);
  });

  it("counts two people who share a display name as two contributors", () => {
    const byKey = new Map([
      [
        "git.prs_merged",
        result({
          a: [["2026-08-01", 1]],
          b: [["2026-08-01", 1]],
        }),
      ],
    ]);
    const namesakes = [
      { person_id: "a", name: "Alex Kim" },
      { person_id: "b", name: "Alex Kim" },
    ];

    expect(bucketBreakdown("git.prs_merged", byKey, namesakes)).toEqual([
      { date: "2026-08-01", total: 2, contributors: ["Alex Kim", "Alex Kim"] },
    ]);
  });

  it("answers with nothing for a metric the response does not carry", () =>
    expect(bucketBreakdown("git.absent", new Map(), MEMBERS)).toEqual([]));
});
