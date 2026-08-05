import { describe, expect, it } from "vitest";

import { buildMetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import {
  ENTITY_ID,
  groupedTimeseriesModel,
  RANGE,
  timeseriesByKey,
} from "@/components/widgets/metric-views/metric-timeseries.test-fixtures";

describe("buildMetricTimeseriesModel", () => {
  it("builds aligned grouped columns and totals", () => {
    const model = groupedTimeseriesModel();
    expect(model.bucket).toBe("week");
    expect(model.buckets).toEqual(["2026-04-20", "2026-04-27", "2026-05-04"]);
    expect(model.columns.map((column) => column.label)).toEqual([
      "org/repo-a",
      "org/repo-b",
    ]);
    expect(model.columns[0]?.points.get("git.commits")?.get("2026-04-27")).toBe(
      0
    );
    expect(model.columns[1]?.points.get("git.commits")?.get("2026-04-27")).toBe(
      null
    );
    expect(model.columns[0]?.totals.get("git.lines_added")).toBe(30);
    expect(model.columns[1]?.totals.get("git.lines_added")).toBe(90);
    expect(model.grandTotals).toEqual([6, 120]);
  });

  it("keeps ranked groups shared and sorts the remainder last", () => {
    const byKey = timeseriesByKey();
    for (const metric of byKey.values()) {
      metric.timeseries?.series.push({
        entity_id: ENTITY_ID,
        dimensions: [],
        total: metric.metric_key === "git.commits" ? 1 : null,
        remainder: true,
        label: "Other",
        points: [{ bucket_start: "2026-04-20", value: 1 }],
      });
    }
    const model = buildMetricTimeseriesModel(
      byKey,
      ["git.commits", "git.lines_added"],
      ENTITY_ID,
      RANGE,
      ["repository"]
    );
    expect(model.columns.map((column) => column.label)).toEqual([
      "org/repo-a",
      "org/repo-b",
      "Other",
    ]);
    expect(model.columns.at(-1)).toMatchObject({
      key: "__remainder__",
      remainder: true,
    });
    expect(model.columns.at(-1)?.totals.get("git.lines_added")).toBeNull();
  });

  it("uses period totals for an ungrouped series", () => {
    const model = buildMetricTimeseriesModel(
      timeseriesByKey(),
      ["git.commits"],
      ENTITY_ID,
      { from: "2026-04-20", to: "2026-04-21" },
      []
    );
    expect(model.buckets).toEqual(["2026-04-20"]);
    expect(model.columns[0]?.totals.get("git.commits")).toBe(6);
  });

  it("leaves grouped totals missing for an older backend response", () => {
    const byKey = timeseriesByKey();
    const metric = byKey.get("git.commits");
    if (!metric?.timeseries) throw new Error("missing fixture timeseries");
    for (const series of metric.timeseries.series) delete series.total;
    const model = buildMetricTimeseriesModel(
      byKey,
      ["git.commits"],
      ENTITY_ID,
      RANGE,
      ["repository"]
    );
    expect(model.columns[0]?.totals.has("git.commits")).toBe(false);
  });

  it("supports month buckets and rejects invalid ranges", () => {
    const byKey = timeseriesByKey();
    const metric = byKey.get("git.commits");
    if (!metric?.timeseries) throw new Error("missing fixture timeseries");
    metric.timeseries.bucket = "month";
    expect(
      buildMetricTimeseriesModel(
        byKey,
        ["git.commits", "missing"],
        ENTITY_ID,
        { from: "2026-01-15", to: "2026-03-02" },
        ["repository"]
      ).buckets
    ).toEqual(["2026-01-01", "2026-02-01", "2026-03-01"]);
    expect(
      buildMetricTimeseriesModel(
        byKey,
        ["git.commits"],
        ENTITY_ID,
        { from: "invalid", to: "2026-03-02" },
        ["repository"]
      ).buckets
    ).toEqual([]);
    expect(
      buildMetricTimeseriesModel(
        byKey,
        ["git.commits"],
        ENTITY_ID,
        { from: "2026-03-03", to: "2026-03-02" },
        ["repository"]
      ).buckets
    ).toEqual([]);
  });
});
