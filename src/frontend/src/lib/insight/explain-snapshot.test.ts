import { describe, expect, it } from "vitest";

import { trendSnapshot } from "./explain-snapshot";
import type {
  SectionTrendPoint,
  SectionTrendSeries,
} from "@/components/portal/section-trend";

const SERIES: SectionTrendSeries[] = [
  { key: "git.commits", label: "Commits", type: "line" },
  { key: "collab.messages_sent", label: "Messages", type: "line" },
];

const DATA: SectionTrendPoint[] = [
  { date: "2026-06-01", "git.commits": 10, "collab.messages_sent": 40 },
  { date: "2026-07-01", "git.commits": 14 },
  { date: "2026-08-01", "git.commits": 9, "collab.messages_sent": 33 },
];

const CONTEXT = {
  title: "Activity over time",
  bucket: "month",
  since: "2026-06-01",
  until: "2026-08-31",
  people: 18,
};

describe("trendSnapshot", () => {
  it("hands over every line the chart draws", () => {
    const snapshot = trendSnapshot(SERIES, DATA, CONTEXT);

    expect(snapshot.series?.map((s) => s.label)).toEqual([
      "Commits",
      "Messages",
    ]);
    expect(snapshot.series?.[0]?.points).toEqual([10, 14, 9]);
  });

  it("keeps a missing bucket as a gap rather than a zero", () => {
    const snapshot = trendSnapshot(SERIES, DATA, CONTEXT);

    expect(snapshot.series?.[1]?.points).toEqual([40, null, 33]);
  });

  it("says the reading belongs to a group, not a person", () => {
    const snapshot = trendSnapshot(SERIES, DATA, CONTEXT);

    expect(snapshot.scope).toBe("organisation");
    expect(snapshot.peer).toBe("Totals across 18 people");
  });

  it("carries the window and the bucket the chart is drawn over", () => {
    const snapshot = trendSnapshot(SERIES, DATA, CONTEXT);

    expect(snapshot.since).toBe("2026-06-01");
    expect(snapshot.until).toBe("2026-08-31");
    expect(snapshot.period).toBe("month");
    expect(snapshot.label).toBe("Activity over time");
  });

  it("leaves the cohort line empty when the rollup covers nobody", () => {
    const snapshot = trendSnapshot(SERIES, DATA, { ...CONTEXT, people: 0 });

    expect(snapshot.peer).toBe("");
  });
});
