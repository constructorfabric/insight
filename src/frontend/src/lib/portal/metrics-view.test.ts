import { describe, expect, it } from "vitest";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";

import {
  chunkKeys,
  compareRows,
  keysWithData,
  metricsViewCatalog,
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
      (key) => key !== "ai.cost",
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

describe("compareRows", () => {
  it("gives one row per metric with each person's value in the given order", () => {
    const byKey = new Map([
      ["git.commits", result("git.commits", [["p1", 3], ["p2", 5]], { short_label: "Commits" })],
      ["git.prs", result("git.prs", [["p2", 1]])],
    ]);

    expect(compareRows(["git.commits", "git.prs", "missing"], byKey, ["p2", "p1"])).toEqual([
      {
        key: "git.commits",
        label: "Commits",
        format: "integer",
        unit: null,
        values: [5, 3],
      },
      {
        key: "git.prs",
        label: "git.prs label",
        format: "integer",
        unit: null,
        values: [1, null],
      },
    ]);
  });
});
