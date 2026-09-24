import { describe, expect, it } from "vitest";

import { nextOrder, ordered, type RowOrder } from "./row-order";

describe("nextOrder", () => {
  it("cycles one column through up, down and back as it came", () => {
    const first = nextOrder(undefined, 1);
    expect(first).toEqual({ column: 1, direction: "ascending" });

    const second = nextOrder(first, 1);
    expect(second).toEqual({ column: 1, direction: "descending" });

    expect(nextOrder(second, 1)).toBeUndefined();
  });

  it("starts the cycle over when another column is clicked", () => {
    const held: RowOrder = { column: 0, direction: "descending" };

    expect(nextOrder(held, 2)).toEqual({ column: 2, direction: "ascending" });
  });
});

describe("ordered", () => {
  const rows = [["b", 2], ["a", 10], ["c", 9]];

  it("returns the rows as they came when nothing is ordered by", () => {
    expect(ordered(rows, undefined)).toBe(rows);
  });

  it("compares numbers as numbers, not as text", () => {
    const up = ordered(rows, { column: 1, direction: "ascending" });

    expect(up.map((row) => row[1])).toEqual([2, 9, 10]);
  });

  it("orders text both ways without disturbing the given rows", () => {
    expect(
      ordered(rows, { column: 0, direction: "ascending" }).map((row) => row[0])
    ).toEqual(["a", "b", "c"]);
    expect(
      ordered(rows, { column: 0, direction: "descending" }).map((row) => row[0])
    ).toEqual(["c", "b", "a"]);
    expect(rows.map((row) => row[0])).toEqual(["b", "a", "c"]);
  });

  // A row with nothing in the column answers no question, either way round.
  it("keeps rows with no value in that column last, whichever way it points", () => {
    const sparse = [[1], [null], [3]];

    expect(
      ordered(sparse, { column: 0, direction: "ascending" }).map((row) => row[0])
    ).toEqual([1, 3, null]);
    expect(
      ordered(sparse, { column: 0, direction: "descending" }).map(
        (row) => row[0]
      )
    ).toEqual([3, 1, null]);
  });
});
