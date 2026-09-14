import { describe, expect, it } from "vitest";

import {
  cellText,
  evidenceRowKeys,
  nextSort,
  summaryLine,
} from "@/lib/metrics/evidence-rows";

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

describe("summaryLine", () => {
  it("passes a single-line value through untouched", () => {
    expect(summaryLine("Add the parser")).toBe("Add the parser");
  });

  it("keeps the subject and drops the body", () => {
    expect(summaryLine("Add the parser\n\nIt handles nested groups.")).toBe(
      "Add the parser"
    );
  });

  it("skips leading blank lines rather than showing an empty cell", () => {
    expect(summaryLine("\n\nOnly a body here")).toBe("Only a body here");
    expect(summaryLine("  \nAdd the parser")).toBe("Add the parser");
  });

  it("returns the text itself when every line is blank", () => {
    expect(summaryLine("  ")).toBe("  ");
  });
});

describe("evidenceRowKeys", () => {
  it("gives a row the same key wherever it sits in the order", () => {
    const rows = [
      { values: { ref: "1", repository: "one", value: 1 } },
      { values: { ref: "2", repository: "two", value: 2 } },
    ];
    const [first] = evidenceRowKeys(rows);
    const reversed = evidenceRowKeys([...rows].reverse());
    expect(reversed[1]).toBe(first);
  });

  it("separates two rows sharing a ref across repositories", () => {
    const keys = evidenceRowKeys([
      { values: { ref: "42", repository: "one" } },
      { values: { ref: "42", repository: "two" } },
    ]);
    expect(keys[0]).not.toBe(keys[1]);
  });

  it("separates rows that match to the last field", () => {
    const keys = evidenceRowKeys([
      { values: { value: 1 } },
      { values: { value: 1 } },
      { values: { value: 1 } },
    ]);
    expect(new Set(keys).size).toBe(3);
  });

  it("does not depend on the order the fields arrived in", () => {
    const [ordered] = evidenceRowKeys([{ values: { a: 1, b: 2 } }]);
    const [reordered] = evidenceRowKeys([{ values: { b: 2, a: 1 } }]);
    expect(ordered).toBe(reordered);
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
