import { describe, expect, it } from "vitest";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";

import { identityPerson, pid } from "@/test/identity";

import {
  chunkKeys,
  compareGaps,
  formatGap,
  headerUnit,
  rolesById,
  keysWithData,
  metricGroups,
  metricsRows,
  metricsViewCatalog,
  sortRows,
  type MetricsRow,
} from "./metrics-view";

function definition(
  key: string,
  over: Partial<MetricDefinition> = {},
): MetricDefinition {
  return {
    metric_key: key,
    entity_type: "person",
    label: key,
    short_label: null,
    description: null,
    explanation: null,
    unit: null,
    format: "integer",
    direction: "higher_is_better",
    dimensions: [],
    is_enabled: true,
    origin: "builtin",
    schema_status: "ok",
    schema_error_code: null,
    first_observed_date: "2026-01-01",
    last_observed_date: "2026-01-31",
    ...over,
  } as MetricDefinition;
}

function result(
  key: string,
  values: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {},
): NormalizedMetricResult {
  return {
    metric_key: key,
    label: `${key} label`,
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

describe("metricsViewCatalog", () => {
  const all = () => true;

  it.each([
    ["a tenant metric", definition("ci.runs", { entity_type: "tenant" })],
    ["a disabled metric", definition("git.off", { is_enabled: false })],
    ["a metric in schema error", definition("git.broken", { schema_status: "error" })],
    [
      "a built-in metric no source has fed",
      definition("git.unfed", { last_observed_date: null }),
    ],
  ])("leaves out %s", (_case, excluded) => {
    const kept = definition("git.commits");

    const offered = metricsViewCatalog([excluded, kept], all);

    expect(offered.map((m) => m.metric_key), `should drop ${excluded.metric_key}`).toEqual([
      "git.commits",
    ]);
  });

  it("keeps a custom metric that has never been observed", () => {
    const custom = definition("custom.x", { origin: "custom", last_observed_date: null });

    expect(metricsViewCatalog([custom], all).map((m) => m.metric_key)).toEqual(["custom.x"]);
  });

  it("leaves out a metric the instance hides", () => {
    const offered = metricsViewCatalog(
      [definition("git.commits"), definition("ai.cost")],
      (metric) => metric.metric_key !== "ai.cost",
    );

    expect(offered.map((m) => m.metric_key)).toEqual(["git.commits"]);
  });

  it("orders metrics by family, as the report builder does", () => {
    const offered = metricsViewCatalog(
      [definition("ai.sessions"), definition("collab.messages"), definition("git.commits")],
      all,
    );

    expect(offered.map((m) => m.metric_key)).toEqual([
      "git.commits",
      "collab.messages",
      "ai.sessions",
    ]);
  });
});

describe("chunkKeys", () => {
  it("splits keys into runs no longer than the size", () => {
    expect(chunkKeys(["a", "b", "c", "d", "e"], 2)).toEqual([["a", "b"], ["c", "d"], ["e"]]);
  });

  it("returns no chunk for no keys", () => {
    expect(chunkKeys([], 50)).toEqual([]);
  });
});

describe("keysWithData", () => {
  it("keeps a metric only when someone in the rows has a finite value", () => {
    const byKey = new Map([
      ["git.commits", result("git.commits", [["p1", 3], ["p2", null]])],
      ["git.prs", result("git.prs", [["p1", null], ["p2", null]])],
      ["ai.cost", result("ai.cost", [["outside", 9]])],
    ]);

    expect(keysWithData(["git.commits", "git.prs", "ai.cost", "missing"], byKey, ["p1", "p2"])).toEqual([
      "git.commits",
    ]);
  });
});

describe("metricsRows", () => {
  const tree = identityPerson("boss", {}, [
    identityPerson("b"),
    identityPerson("a", {}, [identityPerson("c")]),
  ]);
  const shape = (rows: MetricsRow[]) =>
    rows.map((row) => [row.displayName, row.depth, row.hasReports, row.expanded]);

  it("nests reports under their expanded manager, by name", () => {
    const rows = metricsRows(tree, [], new Set([pid("boss"), pid("a")]));

    expect(shape(rows)).toEqual([
      ["boss", 0, true, true],
      ["a", 1, true, true],
      ["c", 2, false, false],
      ["b", 1, false, false],
    ]);
  });

  it("hides the reports of a collapsed manager", () => {
    const rows = metricsRows(tree, [], new Set([pid("boss")]));

    expect(shape(rows)).toEqual([
      ["boss", 0, true, true],
      ["a", 1, true, false],
      ["b", 1, false, false],
    ]);
  });

  it("lists everyone flat and by name when there is no org chart", () => {
    const people = [
      { person_id: pid("z"), display_name: "z", email: "", username: "" },
      { person_id: pid("m"), display_name: "m", email: "", username: "" },
    ];

    expect(shape(metricsRows(null, people, new Set()))).toEqual([
      ["m", 0, false, false],
      ["z", 0, false, false],
    ]);
  });
});

describe("sortRows", () => {
  const row = (name: string): MetricsRow => ({
    entityId: name,
    displayName: name,
    depth: 2,
    hasReports: false,
    expanded: false,
  });
  const byKey = new Map([
    ["git.commits", result("git.commits", [["a", 1], ["b", 3], ["c", null], ["d", 2]])],
  ]);
  const rows = ["a", "b", "c", "d"].map(row);

  it.each([
    ["descending", ["b", "d", "a", "c"]],
    ["ascending", ["a", "d", "b", "c"]],
  ] as const)("orders people by a metric %s, with no value last", (direction, order) => {
    const sorted = sortRows(rows, "git.commits", direction, byKey);

    expect(sorted.map((r) => r.entityId), `should order ${direction}`).toEqual(order);
    expect(sorted.every((r) => r.depth === 0)).toBe(true);
  });
});

describe("metricGroups", () => {
  it("groups neighbouring metrics under their family name", () => {
    expect(metricGroups(["git.commits", "git.prs", "tasks.closed", "ai.cost"])).toEqual([
      { family: "git", name: "Development · Git", keys: ["git.commits", "git.prs"] },
      { family: "tasks", name: "Development · Delivery", keys: ["tasks.closed"] },
      { family: "ai", name: "AI", keys: ["ai.cost"] },
    ]);
  });
});

describe("compareGaps", () => {
  it("marks the biggest value best when higher is better, with everyone else's gap in percent", () => {
    expect(compareGaps([50, 200, null, 100], "higher_is_better")).toEqual([
      { best: false, gap: -75 },
      { best: true, gap: null },
      { best: false, gap: null },
      { best: false, gap: -50 },
    ]);
  });

  it("marks the smallest value best when lower is better, with everyone else's excess in percent", () => {
    expect(compareGaps([83, 698], "lower_is_better")).toEqual([
      { best: true, gap: null },
      { best: false, gap: 741 },
    ]);
  });

  it("marks every person who ties for best", () => {
    expect(compareGaps([5, 5, 8], "lower_is_better").map((cell) => cell.best)).toEqual([
      true,
      true,
      false,
    ]);
  });

  it.each([
    ["a neutral metric", [1, 9], "neutral"],
    ["everyone equal", [5, 5], "higher_is_better"],
    ["a single value", [7, null], "higher_is_better"],
    ["no values", [null, null], "lower_is_better"],
  ] as const)("names no best with %s", (_case, values, direction) => {
    expect(compareGaps(values, direction).some((cell) => cell.best)).toBe(false);
  });

  it("gives no percent when the best value is zero", () => {
    expect(compareGaps([0, 4], "lower_is_better")).toEqual([
      { best: true, gap: null },
      { best: false, gap: null },
    ]);
  });
});

describe("formatGap", () => {
  it.each([
    [null, ""],
    [0, "≈"],
    [63, "+63%"],
    [-47, "−47%"],
  ] as const)("writes a gap of %s as %j", (gap, text) => {
    expect(formatGap(gap)).toBe(text);
  });
});

describe("headerUnit", () => {
  it.each([
    ["a count unit the title does not name", "PR size", "integer", "lines", "lines"],
    ["a count unit the title already names", "Commits per PR", "decimal", "commits", null],
    ["hours, which each cell writes as a duration", "Cycle time", "decimal", "h", null],
    ["a percent, which each cell carries", "Merge rate", "percent", null, null],
    ["no unit", "Commits", "integer", null, null],
  ] as const)("header unit for %s", (_case, label, format, unit, shown) => {
    expect(headerUnit(label, format, unit)).toBe(shown);
  });
});

describe("rolesById", () => {
  it("maps each person in the tree to their job title, skipping blanks", () => {
    const tree = identityPerson("boss", { job_title: "Head of Engineering" }, [
      identityPerson("a", { job_title: " " }),
      identityPerson("b", { job_title: "Software Engineer" }),
    ]);

    expect(rolesById(tree)).toEqual(
      new Map([
        [pid("boss"), "Head of Engineering"],
        [pid("b"), "Software Engineer"],
      ]),
    );
  });
});
