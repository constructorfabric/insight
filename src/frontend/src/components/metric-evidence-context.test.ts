import { describe, expect, it } from "vitest";

import { withOwnTarget } from "@/components/metric-evidence-context";

function target(metricKey: string, label = metricKey) {
  return {
    selection: {
      metric_key: metricKey,
      entity: { type: "person" as const, id: "person-1" },
      period: { from: "2026-07-01", to: "2026-07-31" },
      filters: [],
      display_dimensions: [],
    },
    label,
  };
}

describe("withOwnTarget", () => {
  const scope = [
    target("git.commits"),
    target("git.prs"),
    target("wiki.edits"),
  ];

  it("keeps the caller's own selection, not the scope's copy of it", () => {
    const own = {
      ...target("git.prs", "Pull requests"),
      selection: {
        ...target("git.prs").selection,
        display_dimensions: ["repository"],
      },
    };
    const out = withOwnTarget(scope, own);
    expect(out[1]).toBe(own);
    expect(out[1]?.selection.display_dimensions).toEqual(["repository"]);
  });

  it("leaves the scope's order alone", () => {
    const out = withOwnTarget(scope, target("git.prs"));
    expect(out.map((entry) => entry.selection.metric_key)).toEqual([
      "git.commits",
      "git.prs",
      "wiki.edits",
    ]);
  });

  it("leads with a metric the scope does not carry", () => {
    const own = target("ai.cost");
    const out = withOwnTarget(scope, own);
    expect(out).toHaveLength(4);
    expect(out[0]).toBe(own);
  });

  it("returns just the caller when there is no scope", () => {
    const own = target("git.commits");
    expect(withOwnTarget([], own)).toEqual([own]);
  });
});
