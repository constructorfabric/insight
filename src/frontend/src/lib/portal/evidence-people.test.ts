import { describe, expect, it } from "vitest";

import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { peopleEvidenceView } from "./evidence-people";

const ROSTER = {
  nameByEntity: new Map([
    ["e-ada", "Ada Lovelace"],
    ["e-grace", "Grace Hopper"],
    ["e-alan", "Alan Turing"],
  ]),
  personIdByEntity: new Map([
    ["e-ada", "p-ada"],
    ["e-grace", "p-grace"],
    ["e-alan", "p-alan"],
  ]),
};

function metric(
  values: Array<[string, number | null]>,
  over: Partial<NormalizedMetricResult> = {}
): NormalizedMetricResult {
  return {
    metric_key: "t.commits",
    label: "Commits",
    short_label: "Commits",
    unit: "commits",
    computation: "sum",
    format: "integer",
    direction: "higher_is_better",
    period: {
      view: "period",
      values: values.map(([entity_id, value]) => ({ entity_id, value })),
    },
    ...over,
  } as unknown as NormalizedMetricResult;
}

const DRILLABLE: Partial<NormalizedMetricResult> = {
  drilldown: { granularity: ["event"] },
  selection: {
    metric_key: "t.commits",
    entity: { type: "person", ids: ["e-ada", "e-grace", "e-alan"] },
    period: { from: "2026-07-20", to: "2026-07-26" },
    filters: [],
  },
} as Partial<NormalizedMetricResult>;

describe("peopleEvidenceView", () => {
  it("ranks the busiest first, and names each person's own records", () => {
    const view = peopleEvidenceView(
      metric(
        [
          ["e-ada", 5],
          ["e-grace", 12],
        ],
        DRILLABLE
      ),
      ["e-ada", "e-grace"],
      "Commits · busiest 2 of 9",
      ROSTER
    );

    expect(view.rows.map((row) => row.name)).toEqual([
      "Grace Hopper",
      "Ada Lovelace",
    ]);
    expect(view.rows[0]?.target?.selection.entity).toEqual({
      type: "person",
      id: "e-grace",
    });
    expect(view.rows[0]?.target?.label).toBe("Commits · Grace Hopper");
  });

  it("reads the name and the person id from their own lookups", () => {
    const view = peopleEvidenceView(
      metric([["e-ada", 5]], DRILLABLE),
      ["e-ada"],
      "Commits · 0–50 commits per person",
      ROSTER
    );

    // Two maps of the same shape: this is the assertion that would fail if the
    // roster's lookups were ever passed the wrong way round.
    expect(view.rows[0]?.name).toBe("Ada Lovelace");
    expect(view.rows[0]?.personId).toBe("p-ada");
  });

  it("orders people on the same value by name, so a list never reshuffles", () => {
    const view = peopleEvidenceView(
      metric(
        [
          ["e-grace", 7],
          ["e-alan", 7],
          ["e-ada", 7],
        ],
        DRILLABLE
      ),
      ["e-grace", "e-alan", "e-ada"],
      "Commits · 0–50 commits per person",
      ROSTER
    );

    expect(view.rows.map((row) => row.name)).toEqual([
      "Ada Lovelace",
      "Alan Turing",
      "Grace Hopper",
    ]);
  });

  it("shows the number alone — the column is already headed by the metric", () => {
    const counted = peopleEvidenceView(
      metric([["e-ada", 1234]], DRILLABLE),
      ["e-ada"],
      "Commits · 0–50 commits per person",
      ROSTER
    );
    expect(counted.rows[0]?.valueText).toBe("1,234");
    expect(counted.valueLabel).toBe("Commits");

    // A percent keeps its sign: the sign is the unit, not a suffix beside it.
    const share = peopleEvidenceView(
      metric([["e-ada", 42]], {
        ...DRILLABLE,
        format: "percent",
        unit: null,
        label: "Review coverage",
        short_label: "Coverage",
      } as Partial<NormalizedMetricResult>),
      ["e-ada"],
      "Review coverage · 0–50%",
      ROSTER
    );
    expect(share.rows[0]?.valueText).toBe("42%");
    expect(share.valueLabel).toBe("Coverage");
  });

  it("covers exactly the listed people with its all-records selection", () => {
    const view = peopleEvidenceView(
      metric(
        [
          ["e-ada", 5],
          ["e-grace", null],
        ],
        DRILLABLE
      ),
      ["e-ada", "e-grace"],
      "Commits · 0–50 commits per person",
      ROSTER
    );

    // The person the metric has no value for is in neither the rows nor the
    // records behind them.
    expect(view.rows.map((row) => row.entityId)).toEqual(["e-ada"]);
    expect(view.allRecords?.selection.entity).toEqual({
      type: "persons",
      ids: ["e-ada"],
    });
  });

  it("still lists people when the metric carries no readable evidence", () => {
    const view = peopleEvidenceView(
      metric([["e-ada", 5]]),
      ["e-ada"],
      "Commits · 0–50 commits per person",
      ROSTER
    );

    expect(view.rows).toHaveLength(1);
    expect(view.rows[0]?.target).toBeNull();
    expect(view.allRecords).toBeNull();
  });

  it("carries the metric key the drill is counted under", () => {
    const view = peopleEvidenceView(
      metric([["e-ada", 5]], DRILLABLE),
      ["e-ada"],
      "Commits · 0–50 commits per person",
      ROSTER
    );

    expect(view.metricKey).toBe("t.commits");
  });

  it("names a person the roster cannot, and leaves them unroutable", () => {
    const view = peopleEvidenceView(
      metric([["e-katherine", 3]], DRILLABLE),
      ["e-katherine"],
      "Commits · 0–50 commits per person",
      ROSTER
    );

    expect(view.rows[0]?.name).toBe("e-katherine");
    expect(view.rows[0]?.personId).toBeNull();
  });
});
