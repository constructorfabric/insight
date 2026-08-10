import { describe, expect, it } from "vitest";

import type { CustomMetricGraph } from "@/api/metrics-client";

import {
  draftIsSubmittable,
  draftToGraph,
  EMPTY_DRAFT,
  graphToDraft,
} from "./metric-graph";

const SUM_GRAPH: CustomMetricGraph = {
  metric_key: "example.lines",
  label: "Lines",
  short_label: null,
  description: null,
  explanation: null,
  entity_type: "person",
  unit: "lines",
  format: "integer",
  direction: "higher_is_better",
  computation: "sum",
  scale: null,
  peer_cohort_key: null,
  transform: null,
  source_key: "example_source",
  observation_sql: "SELECT 1",
  measures: ["lines"],
  dimensions: ["repo", "language"],
  inputs: [{ role: "value", measure_key: "lines" }],
};

describe("draftToGraph", () => {
  it("builds a single value input for a sum and omits scale", () => {
    const graph = draftToGraph({
      ...EMPTY_DRAFT,
      metric_key: "example.lines",
      label: "Lines",
      source_key: "example_source",
      observation_sql: "SELECT 1",
      measures: "lines, extra",
      dimensions: "repo, language",
      value_measure: "lines",
      scale: "100",
    });
    expect(graph.inputs).toEqual([{ role: "value", measure_key: "lines" }]);
    expect(graph.measures).toEqual(["lines", "extra"]);
    expect(graph.dimensions).toEqual(["repo", "language"]);
    // scale is dropped for non-ratio computations
    expect(graph.scale).toBeNull();
  });

  it("builds numerator/denominator inputs and keeps scale for a ratio", () => {
    const graph = draftToGraph({
      ...EMPTY_DRAFT,
      metric_key: "example.rate",
      label: "Rate",
      source_key: "example_source",
      observation_sql: "SELECT 1",
      measures: "num, den",
      computation: "ratio",
      numerator_measure: "num",
      denominator_measure: "den",
      scale: "100",
    });
    expect(graph.inputs).toEqual([
      { role: "numerator", measure_key: "num" },
      { role: "denominator", measure_key: "den" },
    ]);
    expect(graph.scale).toBe(100);
  });

  it("collapses an empty transform to null and parses set fields", () => {
    const none = draftToGraph({
      ...EMPTY_DRAFT,
      metric_key: "a.b",
      label: "L",
      source_key: "s",
      observation_sql: "x",
      measures: "m",
      value_measure: "m",
    });
    expect(none.transform).toBeNull();

    const some = draftToGraph({
      ...EMPTY_DRAFT,
      metric_key: "a.b",
      label: "L",
      source_key: "s",
      observation_sql: "x",
      measures: "m",
      value_measure: "m",
      transform_multiplier: "2",
    });
    expect(some.transform).toEqual({
      multiplier: 2,
      offset: null,
      clamp_min: null,
      clamp_max: null,
    });
  });
});

describe("graphToDraft", () => {
  it("round-trips a graph through a draft", () => {
    const draft = graphToDraft(SUM_GRAPH);
    expect(draft.metric_key).toBe("example.lines");
    expect(draft.measures).toBe("lines");
    expect(draft.dimensions).toBe("repo, language");
    expect(draft.value_measure).toBe("lines");
    expect(draftToGraph(draft)).toEqual(SUM_GRAPH);
  });
});

describe("draftIsSubmittable", () => {
  it("requires identity, source, SQL, measures, and value wiring for a sum", () => {
    expect(draftIsSubmittable(EMPTY_DRAFT)).toBe(false);
    expect(
      draftIsSubmittable({
        ...EMPTY_DRAFT,
        metric_key: "a.b",
        label: "L",
        source_key: "s",
        observation_sql: "x",
        measures: "m",
        value_measure: "m",
      })
    ).toBe(true);
  });

  it("requires both legs and a scale for a ratio", () => {
    const base = {
      ...EMPTY_DRAFT,
      metric_key: "a.b",
      label: "L",
      source_key: "s",
      observation_sql: "x",
      measures: "n, d",
      computation: "ratio" as const,
      numerator_measure: "n",
      denominator_measure: "d",
    };
    expect(draftIsSubmittable(base)).toBe(false);
    expect(draftIsSubmittable({ ...base, scale: "100" })).toBe(true);
  });

  it("rejects a non-numeric ratio scale", () => {
    const base = {
      ...EMPTY_DRAFT,
      metric_key: "a.b",
      label: "L",
      source_key: "s",
      observation_sql: "x",
      measures: "n, d",
      computation: "ratio" as const,
      numerator_measure: "n",
      denominator_measure: "d",
    };
    expect(draftIsSubmittable({ ...base, scale: "abc" })).toBe(false);
    expect(draftIsSubmittable({ ...base, scale: "  " })).toBe(false);
  });
});
