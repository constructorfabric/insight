import { describe, expect, it } from "vitest";

import { trendSnapshot, type TrendChartInput } from "./explain-snapshot";

const PRS: TrendChartInput = {
  title: "PRs merged",
  series: [{ key: "git.prs_merged", label: "PRs merged", type: "line" }],
  data: [
    { date: "2026-06-01", "git.prs_merged": 10 },
    { date: "2026-07-01", "git.prs_merged": 14 },
    { date: "2026-08-01", "git.prs_merged": 9 },
  ],
};

const AUTHORS: TrendChartInput = {
  title: "Active contributors · PRs merged",
  series: [{ key: "active", label: "People", type: "line" }],
  data: [
    { date: "2026-06-01", active: 4 },
    { date: "2026-07-01", active: 6 },
    { date: "2026-08-01", active: 3 },
  ],
};

const CONTEXT = {
  title: "Overview · Trend",
  bucket: "month",
  since: "2026-06-01",
  until: "2026-08-31",
  people: 18,
};

describe("trendSnapshot", () => {
  it("hands over every chart on the page, not just the first", () => {
    const snapshot = trendSnapshot([PRS, AUTHORS], CONTEXT);

    expect(snapshot.series?.map((s) => s.label)).toEqual([
      "PRs merged",
      "Active contributors · PRs merged",
    ]);
  });

  it("qualifies each line when one chart draws several", () => {
    const combined: TrendChartInput = {
      title: "Delivery",
      series: [
        { key: "a", label: "Opened", type: "line" },
        { key: "b", label: "Merged", type: "line" },
      ],
      data: [{ date: "2026-06-01", a: 3, b: 2 }],
    };

    const snapshot = trendSnapshot([combined], CONTEXT);

    expect(snapshot.series?.map((s) => s.label)).toEqual([
      "Delivery — Opened",
      "Delivery — Merged",
    ]);
  });

  it("keeps a missing bucket as a gap rather than a zero", () => {
    const gappy: TrendChartInput = {
      ...PRS,
      data: [
        { date: "2026-06-01", "git.prs_merged": 10 },
        { date: "2026-07-01" },
      ],
    };

    const snapshot = trendSnapshot([gappy], CONTEXT);

    expect(snapshot.series?.[0]?.points).toEqual([10, null]);
  });

  it("says the reading belongs to a group, not a person", () => {
    const snapshot = trendSnapshot([PRS], CONTEXT);

    expect(snapshot.scope).toBe("organisation");
    expect(snapshot.peer).toBe("Totals across 18 people");
  });

  it("carries the window and the bucket the charts are drawn over", () => {
    const snapshot = trendSnapshot([PRS], CONTEXT);

    expect(snapshot.since).toBe("2026-06-01");
    expect(snapshot.until).toBe("2026-08-31");
    expect(snapshot.period).toBe("month");
    expect(snapshot.label).toBe("Overview · Trend");
  });

  it("leaves the cohort line empty when the rollup covers nobody", () => {
    const snapshot = trendSnapshot([PRS], { ...CONTEXT, people: 0 });

    expect(snapshot.peer).toBe("");
  });
});
