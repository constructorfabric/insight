import { describe, expect, it } from "vitest";

import { byFamily } from "@/lib/reports/families";

describe("byFamily", () => {
  it("groups metrics in report-navigation order", () => {
    const grouped = byFamily([
      { metric_key: "wiki.pages" },
      { metric_key: "git.commits" },
      { metric_key: "collab.messages" },
      { metric_key: "tasks.closed" },
      { metric_key: "git.prs_merged" },
    ]);

    expect(grouped.map((group) => group.family)).toEqual([
      "git",
      "tasks",
      "collab",
      "wiki",
    ]);
    expect(grouped[0]?.metrics).toHaveLength(2);
  });

  it("uses the product family names", () => {
    expect(byFamily([{ metric_key: "wiki.pages" }])[0]?.name).toBe(
      "Knowledge / Wiki"
    );
  });

  it("keeps unknown families visible", () => {
    const grouped = byFamily([
      { metric_key: "git.commits" },
      { metric_key: "support.tickets" },
    ]);

    expect(grouped.map((group) => group.family)).toEqual(["git", "support"]);
    expect(grouped[1]?.name).toBe("support");
  });
});
