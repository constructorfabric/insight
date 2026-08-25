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
    // The request is built from the block's `metrics`, not from its columns, so
    // a column whose metric is missing here renders empty. Pinned as a pair per
    // total: the comparison is per metric, not "all totals, then all splits".
    expect(timeseries[0]?.metrics).toEqual([
      "git.commits",
      "git.default_branch_commits",
      "git.prs_merged",
      "git.default_branch_prs_merged",
      "git.lines_added",
      "git.lines_removed",
      "git.default_branch_lines_added",
      "git.default_branch_lines_removed",
    ]);
    // Every total is immediately followed by its default-branch reading, so a
    // column pair reads "how much, and how much of it landed".
    expect(timeseries[0]?.table?.columns).toEqual([
      { metric: "git.commits" },
      { metric: "git.default_branch_commits", labelSource: "short" },
      { metric: "git.prs_merged", labelSource: "short" },
      { metric: "git.default_branch_prs_merged", labelSource: "short" },
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
      {
        label: "Lines (default)",
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

  // Looked up by id, not by index: the sibling test above pins positions, and
  // two positional readings of one array is how a later insertion breaks a
  // test that has nothing to do with it.
  it("splits git output by branch scope, uncapped and on the totals", () => {
    const git = groupById("git_output");
    const byScope = git.drilldown.find(
      (block): block is Extract<DrilldownBlock, { id: string }> =>
        "id" in block && block.id === "output-by-branch-scope"
    );
    expect(byScope?.groupBy).toEqual({ default: "branch_scope" });
    // A partition has no long tail, so no limits and no remainder bucket.
    expect(byScope?.groupBy?.limits).toBeUndefined();
    expect(byScope?.metrics).toEqual([
      "git.commits",
      "git.prs_merged",
      "git.lines_added",
      "git.lines_removed",
    ]);

    // The ribbon on a summary card comes from the metric's own breakdown view,
    // and it only renders for a dimension with two groups — which is what
    // `branch_scope` is. The split metrics deliberately do NOT carry it: inside
    // `git.default_branch_commits` the dimension is a constant.
    const scoped = git.collection.metrics
      .filter((metric) =>
        metric.views.some(
          (view) =>
            view.view === "breakdown" &&
            view.dimensions.includes("branch_scope")
        )
      )
      .map((metric) => metric.key);
    expect(scoped).toEqual([
      "git.prs_merged",
      "git.lines_added",
      "git.code_lines",
      "git.prs_created",
    ]);
  });

  it("gives each default-branch split a value and a peer comparison", () => {
    const git = groupById("git_output");
    const byKey = new Map(git.collection.metrics.map((m) => [m.key, m]));

    for (const key of [
      "git.default_branch_commits",
      "git.default_branch_prs_merged",
    ]) {
      // The number alone says nothing: "commits that landed" is only readable
      // against the pool's middle, which is what the peer view carries. The
      // sibling test above pins that neither takes a `branch_scope` breakdown.
      const views = byKey.get(key)?.views.map((view) => view.view);
      expect(views, key).toEqual(expect.arrayContaining(["period", "peer"]));
    }
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
