import { describe, expect, it } from "vitest";

import { parseNavPaths } from "@/lib/portal/nav-policy";
import {
  KPI_ROW_COLLECTION,
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
    expect(groupIdForMetricKey("git.default_branch_prs_merged")).toBe("git_output");
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

  it("caps repository activity on the default branch and keeps line composition grouped by category", () => {
    const git = groupById("git_output");
    const timeseries = git.drilldown.filter(
      (block) => block.view === "timeseries"
    );
    const lines = {
      label: "Lines",
      template: [
        {
          metric: "git.default_branch_lines_added",
          prefix: "+",
          tone: "success",
        },
        { text: " / " },
        {
          metric: "git.default_branch_lines_removed",
          prefix: "−",
          tone: "destructive",
        },
      ],
    };
    expect(timeseries[0]?.groupBy?.limits?.repository).toEqual({
      count: 10,
      rankBy: "git.default_branch_commits",
      includeRemainder: true,
    });
    // The request is built from the block's `metrics`, not from its columns, so
    // a column whose metric is missing here renders empty.
    expect(timeseries[0]?.metrics).toEqual([
      "git.default_branch_commits",
      "git.default_branch_prs_merged",
      "git.default_branch_lines_added",
      "git.default_branch_lines_removed",
    ]);
    expect(timeseries[0]?.table?.columns).toEqual([
      { metric: "git.default_branch_commits" },
      { metric: "git.default_branch_prs_merged", labelSource: "short" },
      lines,
    ]);
    expect(timeseries[1]?.groupBy).toEqual({
      default: "category",
    });
    expect(timeseries[1]?.table?.columns).toEqual([lines]);
  });

  it("gives each default-branch split a value and a peer comparison", () => {
    const git = groupById("git_output");
    const byKey = new Map(git.collection.metrics.map((m) => [m.key, m]));

    for (const key of [
      "git.default_branch_commits",
      "git.default_branch_prs_merged",
    ]) {
      // The number alone says nothing: "commits that landed" is only readable
      // against the pool's middle, which is what the peer view carries.
      const views = byKey.get(key)?.views.map((view) => view.view);
      expect(views, key).toEqual(expect.arrayContaining(["period", "peer"]));
    }
  });
});

describe("every collection asks for a metric key at most once", () => {
  it("holds for every group", () => {
    // `/v1/metric-results` refuses a request naming one key twice, and the
    // refusal is all-or-nothing: a duplicate blanks the whole section rather
    // than dropping one tile. A merge that appends a key another branch had
    // already added is exactly how this slips in.
    for (const group of GROUPS) {
      const keys = group.collection.metrics.map((metric) => metric.key);
      expect(new Set(keys).size, `${group.id} names a key twice: ${keys.join(", ")}`).toBe(
        keys.length,
      );
    }
  });

  it("holds for the KPI row", () => {
    const keys = KPI_ROW_COLLECTION.metrics.map((metric) => metric.key);
    expect(new Set(keys).size).toBe(keys.length);
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
