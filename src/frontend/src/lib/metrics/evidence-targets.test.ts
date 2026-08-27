import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  collectionEvidenceTargets,
  narrowedEvidenceSelection,
} from "@/lib/metrics/evidence-targets";

function metric(
  metricKey: string,
  options: {
    drillable?: boolean;
    selection?: boolean;
    filters?: { dimension: string; values: string[] }[];
  } = {}
): NormalizedMetricResult {
  const { drillable = true, selection = true, filters = [] } = options;
  return {
    metric_key: metricKey,
    label: metricKey.toUpperCase(),
    drilldown: drillable ? { granularity: ["event"] } : undefined,
    selection: selection
      ? {
          metric_key: metricKey,
          entity: { type: "person", ids: ["person-1"] },
          period: { from: "2026-07-01", to: "2026-07-31" },
          filters,
        }
      : undefined,
  } as unknown as NormalizedMetricResult;
}

function byKeyOf(...metrics: NormalizedMetricResult[]) {
  return new Map(metrics.map((entry) => [entry.metric_key, entry]));
}

describe("collectionEvidenceTargets", () => {
  it("keeps the order the collection declares", () => {
    const out = collectionEvidenceTargets(
      ["b", "a"],
      byKeyOf(metric("a"), metric("b")),
      "person-1"
    );
    expect(out.map((target) => target.selection.metric_key)).toEqual([
      "b",
      "a",
    ]);
  });

  it("names each target by the metric's label", () => {
    const out = collectionEvidenceTargets(["a"], byKeyOf(metric("a")), "p");
    expect(out[0]?.label).toBe("A");
  });

  it("lists a metric once even when two blocks declare it", () => {
    const out = collectionEvidenceTargets(
      ["a", "a"],
      byKeyOf(metric("a")),
      "person-1"
    );
    expect(out).toHaveLength(1);
  });

  it("leaves out what cannot be drilled", () => {
    const out = collectionEvidenceTargets(
      ["a", "b", "c", "missing"],
      byKeyOf(
        metric("a"),
        metric("b", { drillable: false }),
        metric("c", { selection: false })
      ),
      "person-1"
    );
    expect(out.map((target) => target.selection.metric_key)).toEqual(["a"]);
  });

  it("takes the period it was given over the metric's own", () => {
    const range = { from: "2026-08-01", to: "2026-08-31" };
    const out = collectionEvidenceTargets(
      ["a"],
      byKeyOf(metric("a")),
      "person-1",
      range
    );
    expect(out[0]?.selection.period).toEqual(range);
  });
});

describe("narrowedEvidenceSelection", () => {
  const roster = ["person-1", "person-2"];

  it("adds the clicked value to the filters the figure already carries", () => {
    const selection = narrowedEvidenceSelection(
      metric("git.commits", {
        filters: [{ dimension: "category", values: ["code"] }],
      }),
      roster,
      { filters: [{ dimension: "repository", value: "src-a:acme/api" }] }
    );

    expect(selection?.filters).toEqual([
      { dimension: "category", values: ["code"] },
      { dimension: "repository", values: ["src-a:acme/api"] },
    ]);
  });

  it("replaces a filter on the dimension the reader just picked", () => {
    const selection = narrowedEvidenceSelection(
      metric("git.commits", {
        filters: [{ dimension: "repository", values: ["src-a:acme/web"] }],
      }),
      roster,
      { filters: [{ dimension: "repository", value: "src-a:acme/api" }] }
    );

    expect(selection?.filters).toEqual([
      { dimension: "repository", values: ["src-a:acme/api"] },
    ]);
  });

  it("shows the narrowed dimension, so a row says which one it belongs to", () => {
    const selection = narrowedEvidenceSelection(metric("git.commits"), roster, {
      filters: [
        { dimension: "repository", value: "src-a:acme/api" },
        { dimension: "branch_scope", value: "default" },
      ],
    });

    expect(selection?.display_dimensions).toEqual([
      "branch_scope",
      "repository",
    ]);
  });

  it("makes a clicked day the whole period", () => {
    const selection = narrowedEvidenceSelection(metric("git.commits"), roster, {
      day: "2026-07-14",
    });

    expect(selection?.period).toEqual({ from: "2026-07-14", to: "2026-07-14" });
  });

  it("keeps the figure's own period when nothing narrowed it", () => {
    const selection = narrowedEvidenceSelection(
      metric("git.commits"),
      roster,
      {}
    );

    expect(selection?.period).toEqual({ from: "2026-07-01", to: "2026-07-31" });
  });

  it("answers null for a metric whose records cannot be read", () => {
    expect(
      narrowedEvidenceSelection(
        metric("git.commits", { drillable: false }),
        roster,
        {
          filters: [{ dimension: "repository", value: "src-a:acme/api" }],
        }
      )
    ).toBeNull();
    expect(narrowedEvidenceSelection(undefined, roster, {})).toBeNull();
  });

  it("ignores a narrowing with no value — a bar with no dimension is not a filter", () => {
    const selection = narrowedEvidenceSelection(metric("git.commits"), roster, {
      filters: [{ dimension: "repository", value: "" }],
    });

    expect(selection?.filters).toEqual([]);
    expect(selection?.display_dimensions).toEqual([]);
  });
});
