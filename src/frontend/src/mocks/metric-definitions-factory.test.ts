import { describe, expect, it } from "vitest";

import catalog from "./metric-catalog.json";
import { buildMetricDefinitions } from "./metric-definitions-factory";

describe("mock metric definitions", () => {
  it("explain a metric in the catalog's own words", () => {
    const entry = catalog.find((metric) => metric.metric_key === "git.pr_size")!;
    const definition = buildMetricDefinitions().find((metric) => metric.metric_key === "git.pr_size");

    expect(definition).toMatchObject({
      label: entry.label,
      short_label: entry.short_label ?? null,
      description: entry.description,
      explanation: entry.explanation,
      direction: "lower_is_better",
    });
  });
});
