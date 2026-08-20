import { describe, expect, it } from "vitest";

import type { MetricDefinition } from "@/api/metric-definitions-client";

import { catalogueHealth } from "./catalogue-health";

function def(over: Partial<MetricDefinition>): MetricDefinition {
  return {
    metric_key: "git.commits",
    label: "Commits",
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
    last_observed_date: "2020-01-01",
    ...over,
  };
}

describe("catalogueHealth", () => {
  it("counts a checked, observed builtin as serving", () => {
    expect(catalogueHealth([def({})])).toMatchObject({ serving: 1 });
  });

  it("does not count a custom metric as unverified, however its schema reads", () => {
    const counts = catalogueHealth([
      def({ origin: "custom", schema_status: "unchecked", last_observed_date: null }),
    ]);

    expect(counts.unverified).toBe(0);
    expect(counts.custom).toBe(1);
  });

  it("does not count a custom metric as awaiting data", () => {
    const counts = catalogueHealth([
      def({ origin: "custom", schema_status: "unchecked", last_observed_date: null }),
    ]);

    expect(counts.awaitingData).toBe(0);
  });

  it("reports a disabled definition as disabled rather than as health", () => {
    const counts = catalogueHealth([def({ is_enabled: false })]);

    expect(counts).toMatchObject({ disabled: 1, serving: 0, broken: 0 });
  });

  it("separates a builtin that never observed anything from one only part-checked", () => {
    const counts = catalogueHealth([
      def({ schema_status: "unchecked", last_observed_date: null }),
      def({ schema_status: "unchecked", last_observed_date: "2020-01-01" }),
    ]);

    expect(counts).toMatchObject({ awaitingData: 1, unverified: 1 });
  });

  it("counts a schema-broken definition as broken", () => {
    expect(catalogueHealth([def({ schema_status: "error" })])).toMatchObject({
      broken: 1,
    });
  });
});
