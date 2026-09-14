import { describe, expect, it } from "vitest";

import { evidenceCarriers, evidenceMetricFor } from "./evidence-via";

describe("evidenceMetricFor", () => {
  it("sends a line count to the commits that carry it", () => {
    expect(evidenceMetricFor("git.default_branch_code_lines")).toBe(
      "git.default_branch_commits"
    );
    expect(evidenceMetricFor("git.non_default_branch_code_lines")).toBe(
      "git.non_default_branch_commits"
    );
  });

  it("leaves a metric that holds its own records alone", () => {
    expect(evidenceMetricFor("git.prs_merged")).toBe("git.prs_merged");
  });

  it("carries every branch reading into its OWN scope's commits", () => {
    // A crossed pair here would answer a default-branch tile with the commits
    // that never reached the trunk, which reads as data loss rather than a
    // wiring mistake — so the pairs are stated, not derived.
    const pairs = [
      ["git.default_branch_code_lines", "git.default_branch_commits"],
      ["git.default_branch_lines_added", "git.default_branch_commits"],
      ["git.default_branch_lines_removed", "git.default_branch_commits"],
      ["git.non_default_branch_code_lines", "git.non_default_branch_commits"],
      ["git.non_default_branch_lines_added", "git.non_default_branch_commits"],
      [
        "git.non_default_branch_lines_removed",
        "git.non_default_branch_commits",
      ],
      ["git.code_lines", "git.commits"],
      ["git.lines_added", "git.commits"],
      ["git.lines_removed", "git.commits"],
    ] as const;
    for (const [key, carrier] of pairs) {
      expect(evidenceMetricFor(key), key).toBe(carrier);
    }
  });
});

describe("evidenceCarriers", () => {
  it("names every metric a request must also ask for, once", () => {
    expect(
      evidenceCarriers([
        "git.default_branch_code_lines",
        "git.default_branch_lines_added",
        "git.prs_merged",
      ]).sort()
    ).toEqual(["git.default_branch_commits"]);
  });

  it("is empty when nothing needs a carrier", () => {
    expect(evidenceCarriers(["git.prs_merged", "tasks.closed"])).toEqual([]);
  });
});
