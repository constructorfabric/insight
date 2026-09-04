import { describe, expect, it } from "vitest";

import { ariaSort, nextSort, sortRows } from "@/lib/gears/sort";

interface Row {
  name: string;
  effort: number | null;
}

const ROWS: Row[] = [
  { name: "beta", effort: 5 },
  { name: "alpha", effort: 30 },
  { name: "gamma", effort: null },
];

const valueOf = (row: Row, key: "name" | "effort") => row[key];

describe("sortRows", () => {
  it("orders numbers by size", () => {
    const sorted = sortRows(ROWS, { key: "effort", direction: "desc" }, valueOf);

    expect(sorted.map((row) => row.effort)).toEqual([30, 5, null]);
  });

  it("keeps rows carrying no value last, whichever way it is sorted", () => {
    const ascending = sortRows(
      ROWS,
      { key: "effort", direction: "asc" },
      valueOf,
    );

    expect(ascending.map((row) => row.effort)).toEqual([5, 30, null]);
  });

  it("orders text alphabetically", () => {
    const sorted = sortRows(ROWS, { key: "name", direction: "asc" }, valueOf);

    expect(sorted.map((row) => row.name)).toEqual(["alpha", "beta", "gamma"]);
  });

  it("leaves the rows it was given untouched", () => {
    sortRows(ROWS, { key: "name", direction: "asc" }, valueOf);

    expect(ROWS.map((row) => row.name)).toEqual(["beta", "alpha", "gamma"]);
  });
});

describe("nextSort", () => {
  it("opens a new column on its largest values", () => {
    expect(nextSort({ key: "name", direction: "asc" }, "effort")).toEqual({
      key: "effort",
      direction: "desc",
    });
  });

  it("turns the active column around on the second click", () => {
    expect(nextSort({ key: "effort", direction: "desc" }, "effort")).toEqual({
      key: "effort",
      direction: "asc",
    });
  });

  it("clears the sort on the third click", () => {
    expect(nextSort({ key: "effort", direction: "asc" }, "effort")).toBeNull();
  });

  it("opens a column again after the sort was cleared", () => {
    expect(nextSort(null, "effort")).toEqual({
      key: "effort",
      direction: "desc",
    });
  });
});

describe("ariaSort", () => {
  it("names the direction of the sorted column only", () => {
    const sort = { key: "effort", direction: "desc" } as const;

    expect(ariaSort(sort, "effort")).toBe("descending");
    expect(ariaSort(sort, "name")).toBe("none");
  });

  it("names no direction once the sort is cleared", () => {
    expect(ariaSort(null, "effort")).toBe("none");
  });
});
