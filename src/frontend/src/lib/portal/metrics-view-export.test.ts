import { describe, expect, it } from "vitest";

import type { MetricFormat } from "@/api/metric-results-client";
import { matrixToCsv, type Matrix } from "@/lib/export/matrix";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type { MetricsRow } from "@/lib/portal/metrics-view";

import {
  metricsCompareMatrix,
  metricsExportFilename,
  metricsTableMatrix,
} from "./metrics-view-export";

const PERIOD = { from: "2026-07-01", to: "2026-07-31" };

function row(entityId: string, displayName: string): MetricsRow {
  return {
    entityId,
    displayName,
    depth: 0,
    hasReports: false,
    expanded: false,
  };
}

function result(
  key: string,
  label: string,
  values: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {}
): NormalizedMetricResult {
  return {
    metric_key: key,
    label,
    unit: null,
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: values.map(([entity_id, value]) => ({ entity_id, value })),
    },
    ...over,
  } as NormalizedMetricResult;
}

function table(
  byKey: Map<string, NormalizedMetricResult>,
  rows: MetricsRow[]
): Matrix {
  return metricsTableMatrix({
    rows,
    keys: [...byKey.keys()],
    byKey,
    roles: new Map(),
    period: PERIOD,
  });
}

describe("metrics table export", () => {
  it("keeps the given people, in that order, with the raw number", () => {
    const matrix = table(
      new Map([
        [
          "git.commits",
          result("git.commits", "Commits", [
            ["ada", 10],
            ["bo", 2.25],
          ]),
        ],
        [
          "collab.messages",
          result("collab.messages", "Messages", [["ada", 4]]),
        ],
      ]),
      [row("ada", "Ada"), row("bo", "Bo")]
    );

    expect(matrix.columns).toEqual([
      "Person",
      "Role",
      "Period",
      "Commits",
      "Messages",
    ]);
    expect(matrix.rows).toEqual([
      ["Ada", "", "2026-07-01 to 2026-07-31", 10, 4],
      ["Bo", "", "2026-07-01 to 2026-07-31", 2.25, null],
    ]);
  });

  it("names the unit in the header and leaves the cell a number", () => {
    const cases: Array<{
      format: MetricFormat;
      unit: string | null;
      label: string;
      header: string;
      value: number;
    }> = [
      {
        format: "integer",
        unit: "lines",
        label: "PR size",
        header: "PR size (lines)",
        value: 237,
      },
      {
        format: "percent",
        unit: null,
        label: "Pass rate",
        header: "Pass rate (%)",
        value: 42.4,
      },
      {
        format: "currency",
        unit: null,
        label: "AI potential cost",
        header: "AI potential cost (USD)",
        value: 1234.5,
      },
      {
        format: "decimal",
        unit: "h",
        label: "Cycle time",
        header: "Cycle time (hours)",
        value: 2.25,
      },
      {
        format: "decimal",
        unit: "h",
        label: "CI hours",
        header: "CI hours",
        value: 1.5,
      },
      {
        format: "percent",
        unit: null,
        label: "Share %",
        header: "Share %",
        value: 8,
      },
    ];

    for (const item of cases) {
      const matrix = table(
        new Map([
          [
            "metric.key",
            result("metric.key", item.label, [["ada", item.value]], {
              format: item.format,
              unit: item.unit,
            }),
          ],
        ]),
        [row("ada", "Ada")]
      );

      expect(matrix.columns[3], item.label).toBe(item.header);
      expect(matrix.rows[0]?.[3], item.label).toBe(item.value);
    }
  });

  it("puts a role in its own column and keeps a zero", () => {
    const matrix = metricsTableMatrix({
      rows: [row("ada", "Ada"), row("bo", "Bo")],
      keys: ["git.commits"],
      byKey: new Map([
        [
          "git.commits",
          result("git.commits", "Commits", [
            ["ada", 0],
            ["bo", null],
          ]),
        ],
      ]),
      roles: new Map([["bo", "Engineer, Staff"]]),
      period: PERIOD,
    });

    expect(matrix.rows).toEqual([
      ["Ada", "", "2026-07-01 to 2026-07-31", 0],
      ["Bo", "Engineer, Staff", "2026-07-01 to 2026-07-31", null],
    ]);
  });

  it("leaves out a metric that has no result", () => {
    const matrix = table(
      new Map([
        ["git.commits", result("git.commits", "Commits", [["ada", 1]])],
      ]),
      [row("ada", "Ada")]
    );

    expect(
      metricsTableMatrix({
        rows: [row("ada", "Ada")],
        keys: ["git.commits", "missing.metric"],
        byKey: new Map([
          ["git.commits", result("git.commits", "Commits", [["ada", 1]])],
        ]),
        roles: new Map(),
        period: PERIOD,
      }).columns
    ).toEqual(matrix.columns);
  });

  it("separates two metrics that would share a header", () => {
    const matrix = table(
      new Map([
        ["git.commits", result("git.commits", "Commits", [["ada", 1]])],
        ["tasks.commits", result("tasks.commits", "Commits", [["ada", 2]])],
      ]),
      [row("ada", "Ada")]
    );

    expect(matrix.columns.slice(3)).toEqual([
      "Development · Git — Commits",
      "Development · Delivery — Commits",
    ]);
    expect(matrix.rows[0]?.slice(3)).toEqual([1, 2]);
  });

  it("writes a formula-like name as text, and quotes a role that contains a comma", () => {
    const csv = matrixToCsv(
      metricsTableMatrix({
        rows: [row("ada", "=Ada")],
        keys: ["git.commits"],
        byKey: new Map([
          ["git.commits", result("git.commits", "Commits", [["ada", 3]])],
        ]),
        roles: new Map([["ada", "Engineer, Staff"]]),
        period: PERIOD,
      })
    );

    expect(csv).toContain("'=Ada");
    expect(csv).toContain('"Engineer, Staff"');
    expect(csv).toContain("3");
    expect(csv).not.toContain("1 h");
  });

  it("drops a non-finite reading instead of writing it as zero", () => {
    const matrix = table(
      new Map([
        [
          "git.commits",
          result("git.commits", "Commits", [["ada", Number.POSITIVE_INFINITY]]),
        ],
      ]),
      [row("ada", "Ada")]
    );

    expect(matrix.rows[0]?.[3]).toBeNull();
  });
});

describe("metrics compare export", () => {
  it("lays the selected people across the visible metrics, as raw numbers", () => {
    const matrix = metricsCompareMatrix({
      people: [
        { entityId: "ada", displayName: "Ada" },
        { entityId: "bo", displayName: "Bo" },
      ],
      keys: ["git.commits", "collab.messages", "tasks.closed"],
      byKey: new Map([
        [
          "git.commits",
          result("git.commits", "Commits", [
            ["ada", 10],
            ["bo", 20],
          ]),
        ],
        ["collab.messages", result("collab.messages", "Messages", [["bo", 6]])],
      ]),
      roles: new Map([["bo", "Engineer"]]),
      period: PERIOD,
    });

    expect(matrix.columns).toEqual([
      "Metric",
      "Period",
      "Ada",
      "Bo (Engineer)",
    ]);
    expect(matrix.rows).toEqual([
      ["Commits", "2026-07-01 to 2026-07-31", 10, 20],
      ["Messages", "2026-07-01 to 2026-07-31", null, 6],
    ]);
  });

  it("numbers people who would otherwise share a column", () => {
    const matrix = metricsCompareMatrix({
      people: [
        { entityId: "a", displayName: "Ada" },
        { entityId: "b", displayName: "Ada" },
      ],
      keys: ["git.commits"],
      byKey: new Map([
        [
          "git.commits",
          result("git.commits", "Commits", [
            ["a", 1],
            ["b", 2],
          ]),
        ],
      ]),
      roles: new Map(),
      period: PERIOD,
    });

    expect(matrix.columns).toEqual(["Metric", "Period", "Ada", "Ada (2)"]);
    expect(matrix.rows[0]?.slice(2)).toEqual([1, 2]);
  });
});

describe("metrics export filename", () => {
  it("names the page and the period", () => {
    expect(metricsExportFilename("metrics", PERIOD, "csv")).toBe(
      "metrics_2026-07-01_2026-07-31.csv"
    );
    expect(metricsExportFilename("metrics-compare", PERIOD, "xlsx")).toBe(
      "metrics-compare_2026-07-01_2026-07-31.xlsx"
    );
  });

  it("strips characters a filename cannot keep", () => {
    expect(
      metricsExportFilename("metrics", { from: " 2026/07/01 ", to: "" }, "csv")
    ).toBe("metrics_20260701_period.csv");
  });
});
