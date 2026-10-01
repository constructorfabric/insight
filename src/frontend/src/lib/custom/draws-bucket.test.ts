import { describe, expect, it } from "vitest";

import { drawsBucket } from "./draws-bucket";

describe("drawsBucket", () => {
  it("is true for a series drawing the injected column on either axis", () => {
    expect(
      drawsBucket({ type: "line", metric: "m", x: "bucket", y: "opened" }),
    ).toBe(true);
    expect(
      drawsBucket({ type: "area", metric: "m", x: "day", y: "bucket" }),
    ).toBe(true);
  });

  it("is true for a table that lists it among its columns", () => {
    expect(
      drawsBucket({ type: "table", metric: "m", columns: ["bucket", "n"] }),
    ).toBe(true);
  });

  it("is true for a stat or a pie that names it", () => {
    expect(drawsBucket({ type: "stat", metric: "m", value: "bucket" })).toBe(
      true,
    );
    expect(
      drawsBucket({ type: "pie", metric: "m", label: "bucket", value: "n" }),
    ).toBe(true);
  });

  it.each([
    { type: "composed", metric: "m", x: "bucket", y: "n", y2: "r" },
    { type: "heatmap", metric: "m", x: "bucket", value: "n" },
    { type: "pulse", metric: "m", x: "bucket", y: "n" },
    { type: "bar", metric: "m", x: "author", y: "n", series: "bucket" },
  ] as const)("is true for a $type that names it", (widget) => {
    expect(drawsBucket(widget)).toBe(true);
  });

  it("is false for a categorical widget that never reads it", () => {
    // A bucketed run would hand a pie one slice per time bucket, so the same
    // category appears many times and the total is wrong.
    expect(
      drawsBucket({ type: "pie", metric: "m", label: "repo", value: "n" }),
    ).toBe(false);
    expect(
      drawsBucket({ type: "bar", metric: "m", x: "author", y: "n" }),
    ).toBe(false);
    expect(
      drawsBucket({ type: "table", metric: "m", columns: ["author", "n"] }),
    ).toBe(false);
    expect(drawsBucket({ type: "stat", metric: "m", value: "n" })).toBe(false);
  });
  it("is false for a stored kind this build does not know", () => {
    expect(drawsBucket({ type: "sankey", metric: "m" } as never)).toBe(false);
  });
});
