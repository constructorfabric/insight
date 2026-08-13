import { describe, expect, it } from "vitest";

import { additiveKeys, isAdditive } from "@/lib/reports/additive";
import {
  MAX_METRICS_PER_REQUEST,
  MAX_VALUES_PER_REQUEST,
  planRequests,
} from "@/lib/reports/batching";
import { byFamily } from "@/lib/reports/families";
import { buildReportTable } from "@/lib/reports/report-table";
import {
  bucketSpan,
  bucketsInRange,
  bucketLabel,
  needsRollup,
  rollUp,
} from "@/lib/reports/rollup";
import type { ReportPerson } from "@/lib/reports/roster-columns";
import type { MetricResult } from "@/api/metric-results-client";

const months = (...pairs: Array<[string, number | null]>) =>
  pairs.map(([bucket_start, value]) => ({ bucket_start, value }));

describe("rollUp", () => {
  it("adds months into a quarter", () => {
    const rolled = rollUp(
      months(["2026-01-01", 10], ["2026-02-01", 5], ["2026-03-01", 1]),
      "quarter",
    );
    expect(rolled.get("2026-Q1")).toBe(16);
  });

  it("keeps a bucket with no reading empty rather than zero", () => {
    // A zero is a measurement. In a file someone will total, the two mean
    // opposite things.
    const rolled = rollUp(months(["2026-01-01", null]), "quarter");
    expect(rolled.get("2026-Q1")).toBeNull();
  });

  it("treats a measured zero as a reading", () => {
    const rolled = rollUp(months(["2026-01-01", null], ["2026-02-01", 0]), "quarter");
    expect(rolled.get("2026-Q1")).toBe(0);
  });

  it("adds nothing at a bucket the server computed itself", () => {
    // A ratio for one month is that month's ratio. Adding two of them would be
    // meaningless, and at these granularities nothing is added: the value is
    // relabelled and passed through, so the whole catalogue can be reported.
    expect(needsRollup("day")).toBe(false);
    expect(needsRollup("week")).toBe(false);
    expect(needsRollup("month")).toBe(false);
    expect(needsRollup("quarter")).toBe(true);
    expect(needsRollup("year")).toBe(true);

    const rolled = rollUp(
      months(["2026-01-01", 40], ["2026-02-01", 60]),
      "month",
    );
    expect([...rolled.values()]).toEqual([40, 60]);
  });

  it("labels each granularity the way a spreadsheet sorts it", () => {
    expect(bucketLabel("2026-05-01", "month")).toBe("2026-05");
    expect(bucketLabel("2026-05-01", "quarter")).toBe("2026-Q2");
    expect(bucketLabel("2026-05-01", "year")).toBe("2026");
    // A day and a week keep the date the server bucketed them by.
    expect(bucketLabel("2026-05-04", "day")).toBe("2026-05-04");
    expect(bucketLabel("2026-05-04", "week")).toBe("2026-05-04");
  });

  it("enumerates every bucket the period covers, once", () => {
    expect(bucketsInRange("2026-01-15", "2026-08-02", "quarter")).toEqual([
      "2026-Q1",
      "2026-Q2",
      "2026-Q3",
    ]);
  });
});

describe("additive", () => {
  it("admits sums and refuses the rest", () => {
    expect(isAdditive("sum")).toBe(true);
    expect(isAdditive("ratio")).toBe(false);
    expect(isAdditive("median")).toBe(false);
    // The trap: formatted as an integer, reads as a counter, and someone
    // active in two months is distinct in each of them.
    expect(isAdditive("distinct_count")).toBe(false);
  });

  it("leaves out a metric the probe did not answer for", () => {
    const catalogue = [{ metric_key: "a" }, { metric_key: "b" }];
    expect(additiveKeys(catalogue, new Map([["a", "sum" as const]]))).toEqual([
      "a",
    ]);
  });
});

describe("planRequests", () => {
  it("never exceeds the metric cap", () => {
    const metrics = Array.from({ length: 120 }, (_, i) => `m${i}`);
    const batches = planRequests(metrics, ["p1"], 12);
    expect(batches.every((b) => b.metricKeys.length <= MAX_METRICS_PER_REQUEST)).toBe(
      true,
    );
    expect(new Set(batches.flatMap((b) => b.metricKeys)).size).toBe(120);
  });

  it("keeps the values one request carries inside the budget", () => {
    // The server's row check never sees the metric count, so satisfying it
    // alone would let a fifty-metric response through unbounded.
    const metrics = Array.from({ length: 50 }, (_, i) => `m${i}`);
    const people = Array.from({ length: 400 }, (_, i) => `p${i}`);
    const buckets = 12;
    for (const batch of planRequests(metrics, people, buckets)) {
      const values = batch.metricKeys.length * batch.entityIds.length * (buckets + 1);
      expect(values).toBeLessThanOrEqual(MAX_VALUES_PER_REQUEST);
    }
  });

  it("covers every person exactly once per metric batch", () => {
    const people = Array.from({ length: 90 }, (_, i) => `p${i}`);
    const batches = planRequests(["a", "b"], people, 12);
    expect(batches.flatMap((b) => b.entityIds).sort()).toEqual([...people].sort());
  });

  it("asks for nothing when nothing is selected", () => {
    expect(planRequests([], ["p1"], 12)).toEqual([]);
    expect(planRequests(["a"], [], 12)).toEqual([]);
  });
});

const person = (over: Partial<ReportPerson> = {}): ReportPerson => ({
  entityId: "p1",
  name: "Jane Doe",
  email: "jane.doe@example.com",
  division: "Platform",
  department: "Core",
  jobTitle: "Engineer",
  managerName: "Sam Smith",
  managerEmail: "sam.smith@example.com",
  status: "active",
  ...over,
});

function seriesResult(
  metricKey: string,
  entityId: string,
  points: Array<[string, number | null]>,
): MetricResult {
  return {
    metric_key: metricKey,
    label: metricKey,
    computation: "sum",
    views: [
      {
        view: "timeseries",
        bucket: "month",
        series: [{ entity_id: entityId, dimensions: [], points: months(...points) }],
      },
    ],
  } as unknown as MetricResult;
}

describe("buildReportTable", () => {
  it("repeats the person's attributes on every bucket row", () => {
    const table = buildReportTable({
      people: [person()],
      metrics: [{ metric_key: "git.commits", label: "Commits" }],
      results: new Map([
        [
          "git.commits",
          seriesResult("git.commits", "p1", [
            ["2026-01-01", 4],
            ["2026-02-01", 6],
            ["2026-04-01", 1],
          ]),
        ],
      ]),
      range: { from: "2026-01-01", to: "2026-06-30" },
      granularity: "quarter",
    });

    expect(table.columns).toEqual([
      "Person",
      "Email",
      "Division",
      "Department",
      "Job title",
      "Manager",
      "Manager email",
      "Status",
      "Period",
      "From",
      "To",
      "Commits",
    ]);
    expect(table.rows).toEqual([
      [
        "Jane Doe",
        "jane.doe@example.com",
        "Platform",
        "Core",
        "Engineer",
        "Sam Smith",
        "sam.smith@example.com",
        "active",
        "2026-Q1",
        // Clipped to the requested period, not the whole quarter: the report
        // starts in January but ends in June.
        "2026-01-01",
        "2026-03-31",
        10,
      ],
      [
        "Jane Doe",
        "jane.doe@example.com",
        "Platform",
        "Core",
        "Engineer",
        "Sam Smith",
        "sam.smith@example.com",
        "active",
        "2026-Q2",
        "2026-04-01",
        "2026-06-30",
        1,
      ],
    ]);
  });

  it("leaves a cell empty where the metric said nothing", () => {
    const table = buildReportTable({
      people: [person()],
      metrics: [
        { metric_key: "a", label: "A" },
        { metric_key: "b", label: "B" },
      ],
      results: new Map([["a", seriesResult("a", "p1", [["2026-01-01", 3]])]]),
      range: { from: "2026-01-01", to: "2026-01-31" },
      granularity: "month",
    });
    expect(table.rows[0]?.at(-2)).toBe(3);
    expect(table.rows[0]?.at(-1)).toBeNull();
  });

  it("holds a row for a person the metrics never mention", () => {
    // Otherwise a reader cannot tell "no activity" from "left out of the file".
    const table = buildReportTable({
      people: [person(), person({ entityId: "p2", name: "Sam Smith" })],
      metrics: [{ metric_key: "a", label: "A" }],
      results: new Map([["a", seriesResult("a", "p1", [["2026-01-01", 3]])]]),
      range: { from: "2026-01-01", to: "2026-01-31" },
      granularity: "month",
    });
    expect(table.rows).toHaveLength(2);
    expect(table.rows[1]?.at(-1)).toBeNull();
  });
});

describe("byFamily", () => {
  it("groups by the key's family, in the order a reader meets them", () => {
    const grouped = byFamily([
      { metric_key: "wiki.pages" },
      { metric_key: "git.commits" },
      { metric_key: "collab.messages" },
      { metric_key: "tasks.closed" },
      { metric_key: "git.prs_merged" },
    ]);
    expect(grouped.map((g) => g.family)).toEqual([
      "git",
      "tasks",
      "collab",
      "wiki",
    ]);
    expect(grouped[0]?.metrics).toHaveLength(2);
  });

  it("names families with the words the Directions zone uses", () => {
    expect(byFamily([{ metric_key: "wiki.pages" }])[0]?.name).toBe(
      "Knowledge / Wiki",
    );
  });

  it("shows a family it has no name for rather than dropping it", () => {
    // A metric added under a new prefix must appear in the picker, not vanish.
    const grouped = byFamily([
      { metric_key: "git.commits" },
      { metric_key: "support.tickets" },
    ]);
    expect(grouped.map((g) => g.family)).toEqual(["git", "support"]);
    expect(grouped[1]?.name).toBe("support");
  });
});

describe("bucketSpan", () => {
  const range = { from: "2026-05-13", to: "2026-08-12" };

  it("clips a bucket to the period the reader asked for", () => {
    // Q2 is April to June, but this report starts in mid-May. Printing the
    // whole quarter's dates would invite comparison with a full quarter.
    expect(bucketSpan("2026-Q2", "quarter", range)).toEqual({
      from: "2026-05-13",
      to: "2026-06-30",
    });
    expect(bucketSpan("2026-Q3", "quarter", range)).toEqual({
      from: "2026-07-01",
      to: "2026-08-12",
    });
  });

  it("gives a month its own days, and the last one its real length", () => {
    expect(bucketSpan("2026-06", "month", range)).toEqual({
      from: "2026-06-01",
      to: "2026-06-30",
    });
    expect(bucketSpan("2026-02", "month", { from: "2026-01-01", to: "2026-12-31" })).toEqual({
      from: "2026-02-01",
      to: "2026-02-28",
    });
  });

  it("gives a week seven days and a day one", () => {
    expect(bucketSpan("2026-06-01", "week", range)).toEqual({
      from: "2026-06-01",
      to: "2026-06-07",
    });
    expect(bucketSpan("2026-06-01", "day", range)).toEqual({
      from: "2026-06-01",
      to: "2026-06-01",
    });
  });
});
