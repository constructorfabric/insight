/**
 * The request planner: the backend takes one view of each kind per metric
 * per request, so conflicting section needs must spread over collections —
 * and every section must still find the view that was requested for it.
 */
import { describe, expect, it } from "vitest";

import type { TenantLensConfig } from "@/lib/portal/lens-configs";

import { planTenantRequests, sectionNeeds } from "./plan";

function lens(sections: TenantLensConfig["sections"]): TenantLensConfig {
  return {
    title: "t",
    entity: "tenant",
    notIngested: "n",
    sections,
  };
}

function viewsOf(
  plan: ReturnType<typeof planTenantRequests>,
  index: number,
  metric: string
) {
  return plan.collections[index]?.metrics.find((m) => m.key === metric)?.views;
}

describe("planTenantRequests", () => {
  it("serves a conflict-free lens from a single collection", () => {
    const config = lens([
      { kind: "headline", metrics: ["ci.gate_pass_rate"] },
      { kind: "trend", title: "t", metrics: ["ci.runs"] },
      {
        kind: "composition",
        metric: "ci.deployments",
        dimension: "environment",
        splitBy: "outcome",
        title: "d",
      },
      { kind: "histogram", metric: "ci.run_duration_min", title: "h", caption: "c" },
    ]);
    const plan = planTenantRequests(config, "day");
    expect(plan.collections).toHaveLength(1);
    expect(plan.halves.metrics).toHaveLength(0);
    expect(viewsOf(plan, 0, "ci.gate_pass_rate")).toEqual([{ view: "period" }]);
    expect(viewsOf(plan, 0, "ci.runs")).toEqual([
      { view: "timeseries", bucket: "day" },
    ]);
    expect(viewsOf(plan, 0, "ci.deployments")).toEqual([
      { view: "breakdown", dimensions: ["environment", "outcome"] },
    ]);
    expect(viewsOf(plan, 0, "ci.run_duration_min")).toEqual([
      { view: "histogram" },
    ]);
  });

  it("spreads conflicting timeseries of one metric over collections", () => {
    const config = lens([
      { kind: "trend", title: "t", metrics: ["ci.runs"] },
      { kind: "stacked-trend", metric: "ci.runs", splitBy: "outcome", title: "s" },
      {
        kind: "small-multiples",
        metric: "ci.runs",
        dimension: "repository",
        title: "m",
        top: 12,
      },
      { kind: "heatmap-hours", metric: "ci.runs", title: "h" },
    ]);
    const plan = planTenantRequests(config, "week");
    expect(plan.collections).toHaveLength(4);
    expect(viewsOf(plan, 0, "ci.runs")).toEqual([
      { view: "timeseries", bucket: "week" },
    ]);
    expect(viewsOf(plan, 1, "ci.runs")).toEqual([
      { view: "timeseries", bucket: "week", dimensions: ["outcome"] },
    ]);
    expect(viewsOf(plan, 2, "ci.runs")).toEqual([
      {
        view: "timeseries",
        bucket: "week",
        dimensions: ["repository"],
        groupLimit: { count: 12, include_remainder: false },
      },
    ]);
    // The heatmap is day-bucketed whatever the lens bucket — the weekday
    // axis needs dates.
    expect(viewsOf(plan, 3, "ci.runs")).toEqual([
      { view: "timeseries", bucket: "day", dimensions: ["hour_block"] },
    ]);
    // Every section still finds its own view.
    for (const section of config.sections) {
      for (const need of sectionNeeds(section, "week")) {
        expect(plan.locate(need)).toBeDefined();
      }
    }
  });

  it("widens mergeable breakdowns instead of duplicating them", () => {
    const config = lens([
      {
        kind: "composition",
        metric: "ci.runs",
        dimension: "repository",
        splitBy: "outcome",
        title: "c",
      },
      { kind: "decomposition", metric: "ci.runs", splitBy: "outcome", title: "d" },
    ]);
    const plan = planTenantRequests(config, "day");
    expect(plan.collections).toHaveLength(1);
    // One request serves both: the decomposition re-aggregates over the
    // extra repository dimension.
    expect(viewsOf(plan, 0, "ci.runs")).toEqual([
      { view: "breakdown", dimensions: ["repository", "outcome"] },
    ]);
    const [decompositionNeed] = sectionNeeds(config.sections[1], "day");
    expect(plan.locate(decompositionNeed)).toEqual({ at: "collection", index: 0 });
  });

  it("keeps exact breakdowns apart — a rate cannot be re-aggregated", () => {
    const config = lens([
      {
        kind: "callout-pair",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "c",
      },
      { kind: "hour-columns", metric: "ci.gate_pass_rate", title: "h" },
    ]);
    const plan = planTenantRequests(config, "day");
    expect(plan.collections).toHaveLength(2);
    expect(viewsOf(plan, 0, "ci.gate_pass_rate")).toEqual([
      { view: "period" },
      { view: "breakdown", dimensions: ["repository"] },
    ]);
    expect(viewsOf(plan, 1, "ci.gate_pass_rate")).toEqual([
      { view: "breakdown", dimensions: ["hour_block"] },
    ]);
  });

  it("coalesces a mergeable slot with an identical exact slot", () => {
    const config = lens([
      { kind: "cumulative", metric: "ci.run_hours", dimension: "pipeline", title: "c" },
      {
        kind: "scatter",
        x: "ci.run_duration_min",
        y: "ci.gate_pass_rate",
        size: "ci.run_hours",
        dimension: "pipeline",
        title: "s",
      },
    ]);
    const plan = planTenantRequests(config, "day");
    // ci.run_hours by [pipeline] is needed twice (merge + exact) but with the
    // same dims — one request serves both.
    const runHourViews = plan.collections.flatMap(
      (c) => c.metrics.find((m) => m.key === "ci.run_hours")?.views ?? []
    );
    expect(runHourViews).toEqual([
      { view: "breakdown", dimensions: ["pipeline"] },
    ]);
  });

  it("routes slope and momentum to the half-window request pair", () => {
    const config = lens([
      {
        kind: "slope",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "s",
      },
      {
        kind: "momentum",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "m",
      },
    ]);
    const plan = planTenantRequests(config, "day");
    expect(plan.halves.metrics).toEqual([
      {
        key: "ci.gate_pass_rate",
        views: [{ view: "breakdown", dimensions: ["repository"] }],
      },
    ]);
    const [first, second] = sectionNeeds(config.sections[0], "day");
    expect(plan.locate(first)).toEqual({ at: "first-half" });
    expect(plan.locate(second)).toEqual({ at: "second-half" });
  });

  it("pins every period view to collection 0, where the delta twin lives", () => {
    const config = lens([
      // The stacked trend claims a ci.runs timeseries slot first…
      { kind: "stacked-trend", metric: "ci.runs", splitBy: "outcome", title: "s" },
      { kind: "trend", title: "t", metrics: ["ci.runs"] },
      // …and the headline arrives last but must still land in collection 0.
      { kind: "headline", metrics: ["ci.runs"] },
    ]);
    const plan = planTenantRequests(config, "day");
    expect(plan.locate({ view: "period", metric: "ci.runs" })).toEqual({
      at: "collection",
      index: 0,
    });
  });

  it("does not locate a view nothing asked for", () => {
    const plan = planTenantRequests(lens([]), "day");
    expect(plan.collections).toEqual([{ metrics: [] }]);
    expect(plan.locate({ view: "period", metric: "ci.runs" })).toBeUndefined();
  });
});
