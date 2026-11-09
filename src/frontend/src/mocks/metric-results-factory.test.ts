import { describe, expect, it } from "vitest";
import type { MetricResultsRequest, PeriodView } from "@/api/metric-results-client";
import { buildMetricResultsResponse, metaFor } from "./metric-results-factory";
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

describe("synthetic values per period", () => {
  const valueIn = (period: MetricResultsRequest["period"]) =>
    (buildMetricResultsResponse({ ...request, period }).metrics[0]!.views[0] as PeriodView)
      .values[0]!.value;

  it("answers a different value when the period changes", () => {
    expect(valueIn({ from: "2026-09-08", to: "2026-09-14" })).not.toBe(
      valueIn({ from: "2026-08-15", to: "2026-09-14" }),
    );
  });

  it("answers the same value for the same period", () => {
    expect(valueIn({ from: "2026-09-08", to: "2026-09-14" })).toBe(
      valueIn({ from: "2026-09-08", to: "2026-09-14" }),
    );
  });
});

describe("synthetic metrics follow the catalog", () => {
  const ids = Array.from(
    { length: 40 },
    (_, i) => `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
  );
  const valuesOf = (metricKey: string) =>
    (
      buildMetricResultsResponse({
        ...request,
        entity: { type: "person", ids },
        metrics: [{ metric_key: metricKey, views: [{ view: "period" }] }],
      }).metrics[0]!.views[0] as PeriodView
    ).values.map((v) => v.value!);

  it("describes a metric with no fixture the way the catalog does", () => {
    expect(metaFor("git.first_review_time_h")).toMatchObject({
      format: "decimal",
      direction: "lower_is_better",
      unit: "h",
    });
  });

  it("keeps a percent metric between 0 and 100", () => {
    const values = valuesOf("git.default_branch_merge_rate");

    expect(values.every((v) => v >= 0 && v <= 100), `out of range: ${values}`).toBe(true);
  });
});

describe("synthetic breakdowns per period", () => {
  it.each(["breakdown", "rollup"] as const)("answers a different %s when the period changes", (view) => {
    const valuesIn = (period: MetricResultsRequest["period"]) =>
      JSON.stringify(
        buildMetricResultsResponse({
          ...request,
          period,
          metrics: [
            { metric_key: "git.commits", views: [{ view, dimensions: ["repository"] }] },
          ],
        }).metrics[0]!.views[0],
      );

    expect(valuesIn({ from: "2026-09-08", to: "2026-09-14" })).not.toBe(
      valuesIn({ from: "2026-08-15", to: "2026-09-14" }),
    );
  });
});

describe("default-branch git metrics", () => {
  it("carry the catalog's labels, units and computation", () => {
    const keys = [
      "git.default_branch_commits",
      "git.default_branch_prs_merged",
      "git.default_branch_prs_created",
      "git.pr_size",
      "git.default_branch_code_lines",
    ];
    const response = buildMetricResultsResponse({
      ...request,
      metrics: keys.map((metric_key) => ({ metric_key, views: [{ view: "period" as const }] })),
    });

    expect(
      response.metrics.map((m) => [m.metric_key, m.label, m.short_label, m.unit, m.computation]),
    ).toEqual([
      ["git.default_branch_commits", "Commits", undefined, "commits", "sum"],
      ["git.default_branch_prs_merged", "PRs merged", "PRs merged", "PRs", "sum"],
      ["git.default_branch_prs_created", "PRs opened", "PRs opened", "PRs", "sum"],
      ["git.pr_size", "PR size", undefined, "lines", "median"],
      ["git.default_branch_code_lines", "Code lines", "Code lines", "lines", "sum"],
    ]);
  });
});
