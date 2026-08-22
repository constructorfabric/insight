/**
 * When a metric's evidence addresses something outside the product.
 *
 * The rule these pin is that a link is only ever built from what a row
 * actually says. Every case below is a way of NOT knowing: a provider whose
 * address we cannot derive, a repository value that only looks like a path, a
 * ref that names neither a commit nor a pull request, a dimension the metric
 * never declared. Each of those has to come back empty rather than guess,
 * because a link that looks right and goes nowhere is worse than no link.
 */
import { describe, expect, it } from "vitest";

import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";
import {
  activityEventLabel,
  evidenceRecordLinks,
  evidenceRefText,
  githubIssueUrl,
  githubRecordUrl,
  githubRepoUrl,
  isGitMetric,
  withSourceDimension,
} from "@/lib/metrics/provider-links";

const SHA1 = "e0f4823a55ac28276cf068f066ebf66f872a059c";
const SHA256 = `${SHA1}${SHA1.slice(0, 24)}`;

describe("githubRepoUrl", () => {
  // The two serving paths spell one connector differently: a breakdown row
  // carries the dimension's VALUE, an evidence row the LABEL it resolves to.
  // Both name GitHub, so both have to be accepted — reading only one of them
  // is why the drilldown silently produced no links at all.
  it.each([
    ["the connector key", "github"],
    ["the display label", "GitHub"],
    ["either, whatever the casing", "GITHUB"],
    ["either, padded", "  github  "],
  ])("accepts %s", (_case, source) => {
    expect(githubRepoUrl(source, "owner/repo")).toBe(
      "https://github.com/owner/repo"
    );
  });

  it.each([
    ["a provider that can live at any address", "gitlab"],
    ["a provider that can live at any address", "bitbucket_cloud"],
    ["a provider by its label", "Bitbucket Cloud"],
    ["nothing at all", null],
    ["nothing at all", undefined],
    ["something adjacent but not it", "github-enterprise"],
  ])("refuses %s (%s)", (_case, source) => {
    expect(githubRepoUrl(source, "owner/repo")).toBeNull();
  });

  it.each([
    // The bug this exists to prevent: gold's `repository_value` is account
    // qualified, and only its LABEL is the addressable path. Feeding the value
    // in produced a link for every row that pointed nowhere.
    ["an account-qualified value rather than the label", "12:owner/repo"],
    ["a slug with no project to pair it with", "repo"],
    ["the placeholder an absent project leaves", "Unknown"],
    ["a path with more segments than a repository has", "owner/repo/extra"],
    ["a path that is only a separator", "/"],
    ["a half-path", "owner/"],
    ["nothing at all", ""],
    ["nothing at all", null],
    ["nothing at all", undefined],
  ])("refuses %s (%s)", (_case, repository) => {
    expect(githubRepoUrl("github", repository)).toBeNull();
  });

  it("keeps the punctuation a repository name is allowed to carry", () => {
    expect(githubRepoUrl("github", "my-org/some.repo_name")).toBe(
      "https://github.com/my-org/some.repo_name"
    );
  });
});

describe("githubRecordUrl", () => {
  const repo = "https://github.com/owner/repo";

  // A commit hash is fixed-length hex and a pull request number is a short
  // decimal, so the ref alone separates them — which is what lets this work
  // without the `record_kind` column the evidence row never projects.
  it.each([
    ["a SHA-1 commit", SHA1],
    ["a SHA-256 commit", SHA256],
    ["a commit in upper case", SHA1.toUpperCase()],
  ])("sends %s to its commit page", (_case, ref) => {
    expect(githubRecordUrl(repo, ref)).toBe(`${repo}/commit/${ref}`);
  });

  it.each([
    ["a pull request", "2696"],
    ["the first pull request", "1"],
  ])("sends %s to its pull-request page", (_case, ref) => {
    expect(githubRecordUrl(repo, ref)).toBe(`${repo}/pull/${ref}`);
  });

  it.each([
    ["a ref that is neither shape", "not-a-ref"],
    ["hex of the wrong length", "abc123"],
    ["hex one short of a SHA-1", SHA1.slice(1)],
    ["a number that is not one", "12.5"],
    ["nothing at all", ""],
    ["nothing at all", null],
    ["nothing at all", undefined],
  ])("refuses %s (%s)", (_case, ref) => {
    expect(githubRecordUrl(repo, ref)).toBeNull();
  });
});

describe("isGitMetric", () => {
  it.each(["git.commits", "git.lines_added", "git.pr_cycle_time_h"])(
    "claims %s",
    (key) => {
      expect(isGitMetric(key)).toBe(true);
    }
  );

  // The family prefix is the gate that keeps every other source out, so a key
  // that merely mentions git is not one.
  it.each(["tasks.closed", "wiki.pages_created", "ai.adoption", "gitlab.x"])(
    "disclaims %s",
    (key) => {
      expect(isGitMetric(key)).toBe(false);
    }
  );
});

describe("withSourceDimension", () => {
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

  it("asks for the source a git metric declares", () => {
    const result = withSourceDimension(
      selection("git.commits"),
      new Set(["repository", "source"])
    );

    expect(result.display_dimensions).toEqual(["source"]);
  });

  it("keeps the requested dimensions sorted, so one selection is one query key", () => {
    const result = withSourceDimension(
      selection("git.commits", ["repository"]),
      new Set(["repository", "source"])
    );

    expect(result.display_dimensions).toEqual(["repository", "source"]);
  });

  // Asking for an undeclared dimension is refused outright, so asking on spec
  // would trade a missing link for a dialog that cannot open at all.
  it("leaves a git metric that declares no source untouched", () => {
    const original = selection("git.commits_per_active_day");

    expect(withSourceDimension(original, new Set())).toBe(original);
  });

  it("leaves everything untouched while the catalogue is unknown", () => {
    const original = selection("git.commits");

    expect(withSourceDimension(original, null)).toBe(original);
    expect(withSourceDimension(original, undefined)).toBe(original);
  });

  it("leaves a metric of a family that links nothing untouched", () => {
    const original = selection("wiki.pages_created");

    expect(withSourceDimension(original, new Set(["source"]))).toBe(original);
  });

  it("asks for source on a task metric too, since its ref links out", () => {
    const original = selection("tasks.closed");

    expect(
      withSourceDimension(original, new Set(["source"])).display_dimensions
    ).toContain("source");
  });

  it("does not ask twice for a source the caller already wanted", () => {
    const original = selection("git.commits", ["source"]);

    expect(withSourceDimension(original, new Set(["source"]))).toBe(
      original
    );
  });
});

describe("evidenceRecordLinks", () => {
  const row = {
    source: "GitHub",
    repository: "owner/repo",
    ref: SHA1,
  };

  it("addresses the repository and the record from one row", () => {
    expect(evidenceRecordLinks("git.commits", row)).toEqual({
      repository: "https://github.com/owner/repo",
      ref: `https://github.com/owner/repo/commit/${SHA1}`,
      title: `https://github.com/owner/repo/commit/${SHA1}`,
    });
  });

  // The id of a record and the human-readable summary of it are two ways of
  // naming the same page, so they carry the same link.
  it("points a record's id and its summary at the same page", () => {
    const links = evidenceRecordLinks("git.commits", { ...row, ref: "2696" });

    expect(links.ref).toBe("https://github.com/owner/repo/pull/2696");
    expect(links.title).toBe(links.ref);
  });

  it("still addresses the repository when the ref names nothing", () => {
    const links = evidenceRecordLinks("git.commits", { ...row, ref: "" });

    expect(links.repository).toBe("https://github.com/owner/repo");
    expect(links.ref).toBeUndefined();
    expect(links.title).toBeUndefined();
  });

  it.each([
    ["the metric belongs to another family", "tasks.closed", row],
    [
      "the provider's address is not derivable",
      "git.commits",
      { ...row, source: "gitlab" },
    ],
    [
      "the row does not say which provider it came from",
      "git.commits",
      { repository: "owner/repo", ref: SHA1 },
    ],
    [
      "the repository is not a path",
      "git.commits",
      { ...row, repository: "Unknown" },
    ],
    ["the row says nothing", "git.commits", {}],
  ])("says nothing when %s", (_case, metricKey, values) => {
    expect(evidenceRecordLinks(metricKey, values)).toEqual({});
  });

  // Row values arrive as `unknown`; a non-string where a string was expected
  // is a reason to say nothing, not to coerce it into a URL.
  it("says nothing when a value is not the string it should be", () => {
    expect(
      evidenceRecordLinks("git.commits", {
        source: "github",
        repository: 42,
        ref: SHA1,
      })
    ).toEqual({});
  });
});

describe("githubIssueUrl", () => {
  it("addresses the issue's own page from the readable key", () => {
    expect(githubIssueUrl("github", "constructorfabric/insight#2717")).toBe(
      "https://github.com/constructorfabric/insight/issues/2717"
    );
  });

  it("accepts the label form the drilldown projects", () => {
    expect(githubIssueUrl("GitHub", "owner/repo#1")).toBe(
      "https://github.com/owner/repo/issues/1"
    );
  });

  it("refuses a tracker whose address is not derivable", () => {
    expect(githubIssueUrl("jira", "PROJ-7")).toBeNull();
    expect(githubIssueUrl("youtrack", "owner/repo#1")).toBeNull();
  });

  it("refuses a key that does not name a repository and a number", () => {
    for (const ref of ["PROJ-7", "4884190007", "owner/repo", "#12", ""]) {
      expect(githubIssueUrl("github", ref)).toBeNull();
    }
  });
});

describe("evidenceRefText", () => {
  it("drops the repository prefix a whole drilldown shares", () => {
    expect(evidenceRefText("tasks.closed", "constructorfabric/insight#2717")).toBe(
      "#2717"
    );
  });

  it("leaves a ref of another shape alone", () => {
    expect(evidenceRefText("tasks.closed", "PROJ-7")).toBe("PROJ-7");
  });

  it("leaves other families alone, pull request numbers included", () => {
    expect(evidenceRefText("git.prs", "2717")).toBe("2717");
  });
});

describe("evidenceRecordLinks for task evidence", () => {
  it("links both the ref and the title at the issue", () => {
    const issue = "https://github.com/owner/repo/issues/12";

    expect(
      evidenceRecordLinks("tasks.closed", {
        source: "github",
        ref: "owner/repo#12",
      })
    ).toEqual({ ref: issue, title: issue });
  });

  it("says nothing when the row does not carry its source", () => {
    expect(evidenceRecordLinks("tasks.closed", { ref: "owner/repo#12" })).toEqual(
      {}
    );
  });
});

describe("activityEventLabel", () => {
  it("names an issue by number and summary", () => {
    expect(
      activityEventLabel("tasks.closed", "owner/repo#12", "Fix the importer")
    ).toBe("#12: Fix the importer");
  });

  it("falls back to whichever half the row has", () => {
    expect(activityEventLabel("tasks.closed", "owner/repo#12", null)).toBe("#12");
    expect(activityEventLabel("tasks.closed", null, "Fix the importer")).toBe(
      "Fix the importer"
    );
    expect(activityEventLabel("tasks.closed", null, null)).toBeNull();
  });

  it("leaves other families reading as they did", () => {
    expect(activityEventLabel("git.commits", "e0f4823", "Fix the importer")).toBe(
      "Fix the importer"
    );
  });
});
