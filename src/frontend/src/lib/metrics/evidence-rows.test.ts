import { describe, expect, it } from "vitest";

import type { MetricEvidenceColumn } from "@/api/metric-drilldown-client";
import {
  cellText,
  nextSort,
  visibleEvidenceRows,
} from "@/lib/metrics/evidence-rows";

const COLUMNS: MetricEvidenceColumn[] = [
  { key: "ref", label: "Ref", type: "string" },
  { key: "title", label: "Title", type: "string" },
  { key: "value", label: "Value", type: "number" },
];

const ROWS = [
  { values: { ref: "a1", title: "Add parser", value: 12 } },
  { values: { ref: "b2", title: "Fix logging", value: 3 } },
  { values: { ref: "c3", title: "Add cache", value: 40 } },
];

describe("cellText", () => {
  it("renders a missing value as a dash rather than an empty cell", () => {
    expect(cellText(null, "string")).toBe("—");
    expect(cellText(undefined, "number")).toBe("—");
  });

  it("formats numbers and leaves strings alone", () => {
    expect(cellText(1500, "number")).toBe("1,500");
    expect(cellText("a1", "string")).toBe("a1");
  });

  it("spells out booleans and serialises objects", () => {
    expect(cellText(true, "string")).toBe("Yes");
    expect(cellText({ a: 1 }, "string")).toBe('{"a":1}');
  });
});

describe("visibleEvidenceRows", () => {
  it("returns every row when nothing is asked of it", () => {
    const out = visibleEvidenceRows({
      rows: ROWS,
      columns: COLUMNS,
      search: "",
      sort: null,
    });
    expect(out).toHaveLength(3);
  });

  it("matches the search against any column, case-insensitively", () => {
    const out = visibleEvidenceRows({
      rows: ROWS,
      columns: COLUMNS,
      search: "ADD",
      sort: null,
    });
    expect(out.map((row) => row.values.ref)).toEqual(["a1", "c3"]);
  });

  it("searches formatted text, so a number reads as it is displayed", () => {
    const rows = [{ values: { ref: "a1", title: "t", value: 1500 } }];
    const out = visibleEvidenceRows({
      rows,
      columns: COLUMNS,
      search: "1,500",
      sort: null,
    });
    expect(out).toHaveLength(1);
  });

  it("sorts numbers numerically, not as text", () => {
    const out = visibleEvidenceRows({
      rows: ROWS,
      columns: COLUMNS,
      search: "",
      sort: { key: "value", direction: "asc" },
    });
    expect(out.map((row) => row.values.value)).toEqual([3, 12, 40]);
  });

  it("reverses on a descending sort", () => {
    const out = visibleEvidenceRows({
      rows: ROWS,
      columns: COLUMNS,
      search: "",
      sort: { key: "value", direction: "desc" },
    });
    expect(out.map((row) => row.values.value)).toEqual([40, 12, 3]);
  });

  it("keeps rows with no value last in both directions", () => {
    const rows = [
      { values: { ref: "a", value: 5 } },
      { values: { ref: "b", value: null } },
      { values: { ref: "c", value: 1 } },
    ];
    for (const direction of ["asc", "desc"] as const) {
      const out = visibleEvidenceRows({
        rows,
        columns: COLUMNS,
        search: "",
        sort: { key: "value", direction },
      });
      expect(out.at(-1)?.values.ref).toBe("b");
    }
  });

  it("does not reorder the array it was given", () => {
    const rows = [...ROWS];
    visibleEvidenceRows({
      rows,
      columns: COLUMNS,
      search: "",
      sort: { key: "value", direction: "asc" },
    });
    expect(rows.map((row) => row.values.ref)).toEqual(["a1", "b2", "c3"]);
  });

  it("applies the search before the sort", () => {
    const out = visibleEvidenceRows({
      rows: ROWS,
      columns: COLUMNS,
      search: "add",
      sort: { key: "value", direction: "desc" },
    });
    expect(out.map((row) => row.values.ref)).toEqual(["c3", "a1"]);
  });
});

describe("nextSort", () => {
  it("starts a new column ascending", () => {
    expect(nextSort(null, "value")).toEqual({ key: "value", direction: "asc" });
    expect(nextSort({ key: "ref", direction: "desc" }, "value")).toEqual({
      key: "value",
      direction: "asc",
    });
  });

  it("cycles the same column through descending and back to unsorted", () => {
    const ascending = nextSort(null, "value");
    const descending = nextSort(ascending, "value");
    expect(descending).toEqual({ key: "value", direction: "desc" });
    expect(nextSort(descending, "value")).toBeNull();
  });
});
