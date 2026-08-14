import { describe, expect, it } from "vitest";

import { dropRedundantMetrics } from "./metric-containment";

const item = (key: string) => ({ key });

describe("dropRedundantMetrics", () => {
  it("drops the wider metric when the narrower one is also on screen", () => {
    // "Lines added 229" above "Code lines added 73" reads as two findings.
    // It is one, counted twice, and the reader is left to notice that.
    const kept = dropRedundantMetrics([
      item("git.lines_added"),
      item("git.code_lines"),
    ]);
    expect(kept.map((item) => item.key)).toEqual(["git.code_lines"]);
  });

  it("keeps the wider metric when it is the only one there", () => {
    // Nothing is duplicated, and the wider metric is still a real finding.
    const kept = dropRedundantMetrics([item("git.lines_added")]);
    expect(kept.map((item) => item.key)).toEqual(["git.lines_added"]);
  });

  it("drops a total when any one of its parts is present", () => {
    const kept = dropRedundantMetrics([
      item("collab.files_shared"),
      item("collab.files_shared_internal"),
    ]);
    expect(kept.map((item) => item.key)).toEqual([
      "collab.files_shared_internal",
    ]);
  });

  it("leaves unrelated metrics alone and keeps their order", () => {
    const kept = dropRedundantMetrics([
      item("git.commits"),
      item("collab.meeting_hours"),
      item("tasks.closed"),
    ]);
    expect(kept.map((item) => item.key)).toEqual([
      "git.commits",
      "collab.meeting_hours",
      "tasks.closed",
    ]);
  });

  it("keeps one of several metrics that restate each other", () => {
    // Share of the day outside meetings, hours in meetings, and days with no
    // meetings are one measurement read three ways. Three rows would be three
    // marks on one fact, and a reader counts marks.
    const kept = dropRedundantMetrics([
      item("collab.focus_time_pct"),
      item("collab.meeting_hours"),
      item("collab.meeting_free_days"),
    ]);
    expect(kept.map((item) => item.key)).toEqual(["collab.focus_time_pct"]);
  });

  it("keeps the first, so callers decide by putting the stronger one first", () => {
    const kept = dropRedundantMetrics([
      item("collab.meeting_hours"),
      item("collab.focus_time_pct"),
    ]);
    expect(kept.map((item) => item.key)).toEqual(["collab.meeting_hours"]);
  });

  it("says nothing a more prominent surface has already said", () => {
    // The headline row above the block is where the reader meets it first;
    // repeating the same measurement under a different name further down the
    // page is the same finding twice, unrecognisably.
    const kept = dropRedundantMetrics(
      [item("collab.meeting_hours"), item("git.commits")],
      new Set(["collab.focus_time_pct"])
    );
    expect(kept.map((item) => item.key)).toEqual(["git.commits"]);
  });

  it("also defers to a narrower metric shown elsewhere", () => {
    const kept = dropRedundantMetrics(
      [item("git.lines_added")],
      new Set(["git.code_lines"])
    );
    expect(kept).toEqual([]);
  });
});
