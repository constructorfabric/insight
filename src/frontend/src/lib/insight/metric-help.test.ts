import { describe, expect, it } from "vitest";

import { metricHelp } from "./metric-help";

describe("metricHelp", () => {
  it("carries both fields when the catalog supplies them", () => {
    expect(
      metricHelp({ description: "What it is", explanation: "How it counts" }),
    ).toEqual({ description: "What it is", explanation: "How it counts" });
  });

  it("returns null when there is nothing to say", () => {
    // A surface with no copy must render no tooltip: an empty bubble is worse
    // than no affordance, because it promises an answer.
    expect(metricHelp({})).toBeNull();
    expect(metricHelp({ description: "   ", explanation: "" })).toBeNull();
  });

  it("drops an explanation that only repeats the description", () => {
    expect(metricHelp({ description: "Same", explanation: "Same" })).toEqual({
      description: "Same",
      explanation: null,
    });
  });

  it("keeps one field when only the other is missing", () => {
    expect(metricHelp({ explanation: "How it counts" })).toEqual({
      description: null,
      explanation: "How it counts",
    });
  });
});
