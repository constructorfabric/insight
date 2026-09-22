import { describe, expect, it } from "vitest";

import { pathSegments } from "./dataset-path";

describe("pathSegments", () => {
  it.each([
    ["a", ["a"]],
    ["stats.lines", ["stats", "lines"]],
    // A payload key may hold a dot; the declaration escapes it, and so must
    // the reader, or the table disagrees with every metric over the field.
    ["a\\.b", ["a.b"]],
    ["outer.a\\.b.inner", ["outer", "a.b", "inner"]],
    ["a\\\\b", ["a\\b"]],
  ])("splits %s the way the service does", (path, expected) => {
    expect(pathSegments(path)).toEqual(expected);
  });
});
