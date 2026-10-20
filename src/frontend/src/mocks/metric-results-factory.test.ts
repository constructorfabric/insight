import { describe, expect, it } from "vitest";
import type { MetricResultsRequest, PeriodView } from "@/api/metric-results-client";
import { buildMetricResultsResponse } from "./metric-results-factory";
import { normalizeMetricResults, projectComparison } from "@/lib/metrics/collection";

const request: MetricResultsRequest = {
  entity: { type: "person", ids: ["00000000-0000-4000-8000-000000000001"] },
  period: { from: "2026-09-08", to: "2026-09-14" },
  metrics: [{ metric_key: "git.commits", views: [{ view: "period" }] }],
};

describe("synthetic period comparisons", () => {
  it("provides distinct comparison values that the UI can project", () => {
    const response = buildMetricResultsResponse({ ...request, compare_to: { from: "2026-09-01", to: "2026-09-07" } });
    const view = response.metrics[0]!.views[0] as PeriodView;
    expect(view.values[0]!.compare_to).toEqual(expect.any(Number));
    expect(view.values[0]!.compare_to).not.toBe(view.values[0]!.value);
    expect(projectComparison(normalizeMetricResults(response.metrics)).size).toBe(1);
  });

  it("omits comparison values when no comparison was requested", () => {
    const view = buildMetricResultsResponse(request).metrics[0]!.views[0] as PeriodView;
    expect(view.values[0]).not.toHaveProperty("compare_to");
  });
});
