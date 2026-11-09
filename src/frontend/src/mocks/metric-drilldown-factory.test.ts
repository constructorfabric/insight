import { describe, expect, it } from "vitest";

import type { MetricDrilldownRequest } from "@/api/metric-drilldown-client";

import { drilldownColumns } from "./metric-drilldown-factory";

function request(metricKey: string): MetricDrilldownRequest {
  return {
    metric_key: metricKey,
    entity: { type: "person", id: "00000000-0000-4000-8000-000000000001" },
    period: { from: "2026-10-01", to: "2026-10-31" },
    filters: [],
    display_dimensions: [],
  } as unknown as MetricDrilldownRequest;
}

describe("mock drilldown columns", () => {
  it.each([
    ["a duration", "git.pr_cycle_time_h", true],
    ["a size", "git.pr_size", true],
    ["a count", "git.commits", false],
  ] as const)("gives %s its own value column: %s", (_case, metricKey, hasValue) => {
    const keys = drilldownColumns(request(metricKey)).map((column) => column.key);

    expect(keys.includes("value"), `value column for ${metricKey}`).toBe(hasValue);
  });
});
