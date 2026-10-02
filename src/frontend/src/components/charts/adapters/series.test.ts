import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/custom-client";

import { pairedRows, pointGroups, seriesRows, shareRows } from "./series";

function result(columns: string[], rows: unknown[][]): MetricResult {
  return { columns, rows };
}

describe("seriesRows", () => {
  it("keeps one value per x in row order when no series is named", () => {
    const drawn = seriesRows(
      result(
        ["day", "n"],
        [
          ["2026-01-01", 3],
          ["2026-01-02", "4"],
        ]
      ),
      "day",
      "n"
    );

    expect(drawn.keys).toEqual([{ key: "s0", label: "n" }]);
    expect(drawn.rows).toEqual([
      { x: "2026-01-01", s0: 3 },
      { x: "2026-01-02", s0: 4 },
    ]);
  });

  it.each([
    ["null", null],
    ["text", "n/a"],
    ["NaN", Number.NaN],
    ["empty string", ""],
  ])("leaves a %s cell as a gap rather than a number", (_name, cell) => {
    const drawn = seriesRows(
      result(["day", "n"], [["2026-01-01", cell]]),
      "day",
      "n"
    );

    expect(drawn.rows).toEqual([{ x: "2026-01-01", s0: null }]);
  });

  it("pivots a series column into one key per value, in order of first appearance", () => {
    const drawn = seriesRows(
      result(
        ["day", "repo", "n"],
        [
          ["d1", "api", 1],
          ["d1", "web", 2],
          ["d2", "api", 3],
        ]
      ),
      "day",
      "n",
      "repo"
    );

    expect(drawn.keys).toEqual([
      { key: "s0", label: "api" },
      { key: "s1", label: "web" },
    ]);
    expect(drawn.rows).toEqual([
      { x: "d1", s0: 1, s1: 2 },
      { x: "d2", s0: 3, s1: null },
    ]);
  });

  it("keeps the six largest series and sums the rest into Other", () => {
    const rows = ["a", "b", "c", "d", "e", "f", "g", "h"].map((name, index) => [
      "d1",
      name,
      index + 1,
    ]);

    const drawn = seriesRows(
      result(["day", "who", "n"], rows),
      "day",
      "n",
      "who"
    );

    expect(drawn.keys.map((key) => key.label)).toEqual([
      "c",
      "d",
      "e",
      "f",
      "g",
      "h",
      "Other",
    ]);
    expect(drawn.rows[0]?.other).toBe(1 + 2);
  });

  it("names a missing series value (none)", () => {
    const drawn = seriesRows(
      result(["day", "who", "n"], [["d1", null, 5]]),
      "day",
      "n",
      "who"
    );

    expect(drawn.keys).toEqual([{ key: "s0", label: "(none)" }]);
  });

  it("carries extra columns beside each x under their own keys", () => {
    const drawn = seriesRows(
      result(
        ["day", "n", "goal"],
        [
          ["d1", 1, 10],
          ["d2", 2, null],
        ]
      ),
      "day",
      "n",
      undefined,
      "goal"
    );

    expect(drawn.rows).toEqual([
      { x: "d1", s0: 1, target: 10 },
      { x: "d2", s0: 2, target: null },
    ]);
  });
});

describe("shareRows", () => {
  it("turns each label's series into percentages that sum to a hundred", () => {
    const drawn = shareRows(
      result(
        ["channel", "kind", "n"],
        [
          ["search", "new", 30],
          ["search", "returning", 10],
          ["email", "new", 0],
          ["email", "returning", 0],
        ]
      ),
      "channel",
      "n",
      "kind"
    );

    expect(drawn.rows).toEqual([
      { x: "search", s0: 75, s1: 25 },
      { x: "email", s0: 0, s1: 0 },
    ]);
  });
});

describe("pointGroups", () => {
  it("groups points by series and drops a point missing either coordinate", () => {
    const groups = pointGroups(
      result(
        ["files", "lines", "who"],
        [
          [1, 10, "a"],
          [2, null, "a"],
          [3, 30, "b"],
        ]
      ),
      "files",
      "lines",
      { series: "who" }
    );

    expect(groups).toEqual([
      { key: "s0", label: "a", points: [{ x: 1, y: 10 }] },
      { key: "s1", label: "b", points: [{ x: 3, y: 30 }] },
    ]);
  });

  it("sizes each point by the size column when one is named", () => {
    const groups = pointGroups(
      result(["files", "lines", "goal"], [[1, 10, 4]]),
      "files",
      "lines",
      { size: "goal" }
    );

    expect(groups).toEqual([
      { key: "s0", label: "lines", points: [{ x: 1, y: 10, size: 4 }] },
    ]);
  });
});

describe("shareRows over many labels", () => {
  it("keeps the largest labels and folds the rest into one Other row", () => {
    const rows = Array.from({ length: 15 }, (_, index) => [
      `repo-${index}`,
      "new",
      100 - index,
    ]);

    const drawn = shareRows(
      result(["repo", "kind", "n"], rows),
      "repo",
      "n",
      "kind",
      8
    );

    expect(drawn.rows.map((row) => row.x)).toEqual([
      ...Array.from({ length: 8 }, (_, index) => `repo-${index}`),
      "Other",
    ]);
  });
});

describe("pairedRows", () => {
  it("sums both columns over the rows that share an x", () => {
    const drawn = pairedRows(
      result(
        ["day", "repo", "n", "files"],
        [
          ["d1", "api", 40, 3],
          ["d1", "web", 20, 5],
          ["d2", "api", 60, 4],
        ]
      ),
      "day",
      "n",
      "files"
    );

    expect(drawn).toEqual([
      { x: "d1", y: 60, y2: 8 },
      { x: "d2", y: 60, y2: 4 },
    ]);
  });
});
