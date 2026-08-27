import { describe, expect, it } from "vitest";

import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";
import {
  activityEventLabel,
  evidenceRefText,
  isTaskMetric,
  withTypeDimension,
} from "@/lib/metrics/provider-links";

function selection(
  metricKey: string,
  displayDimensions: string[] = []
): MetricEvidenceSelection {
  return {
    metric_key: metricKey,
    entity: { type: "person", id: "person-1" },
    period: { from: "2026-01-01", to: "2026-01-31" },
    filters: [],
    display_dimensions: displayDimensions,
  };
}

describe("withTypeDimension", () => {
  it("asks for task types when declared", () => {
    expect(
      withTypeDimension(selection("tasks.closed"), new Set(["type"]))
        .display_dimensions
    ).toEqual(["type"]);
  });

  it("leaves other metric families untouched", () => {
    const original = selection("git.commits");

    expect(withTypeDimension(original, new Set(["type"]))).toBe(original);
  });
});

describe("evidenceRefText", () => {
  it("shortens GitHub-style issue refs without changing the copy value", () => {
    expect(evidenceRefText("tasks.closed", "owner/repo#12")).toBe("#12");
  });

  it("keeps other reference formats intact", () => {
    expect(evidenceRefText("tasks.closed", "PROJ-7")).toBe("PROJ-7");
  });
});

describe("activityEventLabel", () => {
  it("names task events by reference and title", () => {
    expect(
      activityEventLabel("tasks.closed", "owner/repo#12", "Fix login")
    ).toBe("#12: Fix login");
  });

  it("uses the title for non-task events", () => {
    expect(activityEventLabel("git.commits", "abc", "Fix login")).toBe(
      "Fix login"
    );
  });
});

describe("isTaskMetric", () => {
  it("recognizes the task metric family", () => {
    expect(isTaskMetric("tasks.closed")).toBe(true);
    expect(isTaskMetric("git.commits")).toBe(false);
  });
});
