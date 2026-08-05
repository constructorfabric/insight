import { describe, expect, it } from "vitest";

import {
  GROUPS,
  groupIdForMetricKey,
  type GroupId,
  type MetricGroup,
} from "@/lib/insight/groups";

function groupById(id: GroupId): MetricGroup {
  const def = GROUPS.find((g) => g.id === id);
  if (!def) throw new Error(`Unknown group: ${id}`);
  return def;
}

describe("groups registry", () => {
  it("groupIdForMetricKey resolves a metric to its owning group, null otherwise", () => {
    expect(groupIdForMetricKey("ai.active_days")).toBe("ai_adoption");
    expect(groupIdForMetricKey("git.prs_merged")).toBe("git_output");
    expect(groupIdForMetricKey("git.pr_cycle_time_h")).toBe("git_output");
    expect(groupIdForMetricKey("tasks.closed")).toBe("task_delivery");
    expect(groupIdForMetricKey("nope.unknown")).toBeNull();
  });

  it("exposes git_output with a histogram drilldown block", () => {
    const git = groupById("git_output");
    expect(git.collection.metrics.length).toBeGreaterThan(0);
    expect(git.drilldown.some((b) => b.view === "histogram")).toBe(true);
  });

  it("combines compatible task throughput metrics in one chart", () => {
    const taskDelivery = groupById("task_delivery");
    const throughput = taskDelivery.drilldown.find(
      (block) => block.view === "timeseries" && block.id === "task-throughput"
    );
    expect(throughput?.chart).toEqual({ multiMetric: "combined" });
  });

  it("caps repository activity and keeps line composition grouped by category", () => {
    const git = groupById("git_output");
    const timeseries = git.drilldown.filter(
      (block) => block.view === "timeseries"
    );
    expect(timeseries[0]?.groupBy?.limits?.repository).toEqual({
      count: 10,
      rankBy: "git.commits",
      includeRemainder: true,
    });
    expect(timeseries[0]?.table?.columns).toEqual([
      { metric: "git.commits" },
      { metric: "git.prs_merged", labelSource: "short" },
      {
        label: "Lines",
        template: [
          { metric: "git.lines_added", prefix: "+", tone: "success" },
          { text: " / " },
          {
            metric: "git.lines_removed",
            prefix: "−",
            tone: "destructive",
          },
        ],
      },
    ]);
    expect(timeseries[1]?.groupBy).toEqual({
      default: "category",
    });
    expect(timeseries[1]?.table?.columns).toEqual([
      {
        label: "Lines",
        template: [
          { metric: "git.lines_added", prefix: "+", tone: "success" },
          { text: " / " },
          {
            metric: "git.lines_removed",
            prefix: "−",
            tone: "destructive",
          },
        ],
      },
    ]);
  });
});
