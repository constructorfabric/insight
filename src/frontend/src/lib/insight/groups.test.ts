import { describe, expect, it } from "vitest";

import { parseNavPaths } from "@/lib/portal/nav-policy";
import {
  GROUPS,
  groupIdForMetricKey,
  visibleGroups,
  type DrilldownBlock,
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

  it("splits closed issues by type, capped with a remainder group", () => {
    const taskDelivery = groupById("task_delivery");
    const byType = taskDelivery.drilldown.find(
      (block): block is Extract<DrilldownBlock, { view: "timeseries" }> =>
        block.view === "timeseries" && block.id === "closed-by-type"
    );
    expect(byType?.metrics).toEqual(["tasks.closed"]);
    expect(byType?.groupBy).toEqual({
      default: "type",
      limits: {
        type: { count: 10, rankBy: "tasks.closed", includeRemainder: true },
      },
    });
    expect(byType?.table?.columns).toEqual([{ metric: "tasks.closed" }]);
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

describe("visibleGroups", () => {
  const gate = (planned: string[], hide: string[] = []) => ({
    hide: parseNavPaths(hide, "hide"),
    planned: parseNavPaths(planned, "planned"),
  });

  it("returns the registry itself when the install gates nothing", () => {
    expect(visibleGroups(false, gate(["zone:scorecard"]))).toBe(GROUPS);
  });

  it("drops a group whose every metric is gated", () => {
    const groups = visibleGroups(false, gate(["metric:ai.*"]));

    expect(groups.map((g) => g.id)).not.toContain("ai_adoption");
    expect(groups.map((g) => g.id)).toContain("task_delivery");
  });

  it("drops a group whose Person-zone section the install marks planned", () => {
    const groups = visibleGroups(false, gate(["zone:person/section:wiki"]));

    expect(groups.map((g) => g.id)).not.toContain("wiki");
  });

  it("takes a gated metric out of the collection, the card and the drilldown", () => {
    const groups = visibleGroups(false, gate(["metric:tasks.resolution_time"]));
    const tasks = groups.find((g) => g.id === "task_delivery");

    expect(tasks?.collection.metrics.map((m) => m.key)).not.toContain(
      "tasks.resolution_time"
    );
    expect(tasks?.card.preview).not.toContain("tasks.resolution_time");
    expect(
      tasks?.drilldown.flatMap((b) => ("metrics" in b ? b.metrics : []))
    ).not.toContain("tasks.resolution_time");
  });

  it("drops a drilldown block left with no metric of its own", () => {
    const before = groupById("task_delivery").drilldown.length;
    const after = visibleGroups(
      false,
      gate(["metric:tasks.resolution_time", "metric:tasks.pickup_time"])
    ).find((g) => g.id === "task_delivery")?.drilldown.length;

    expect(after).toBe(before - 2);
  });

  it("shows everything gated as planned to a reader who asked for it", () => {
    const groups = visibleGroups(
      true,
      gate(["metric:ai.*", "zone:person/section:wiki"])
    );

    expect(groups).toEqual(GROUPS);
  });
});
