import { describe, expect, it } from "vitest";

import { effectiveOrder, nextOrder } from "./usage-sort";

type Key = "visits" | "page_views" | "last_seen";

const DEFAULT: Key = "visits";

describe("nextOrder", () => {
  it.each([
    ["a new column starts at its largest", null, "last_seen", { sort: "last_seen", direction: "desc" }],
    ["the same column flips", { sort: "last_seen", direction: "desc" }, "last_seen", { sort: "last_seen", direction: "asc" }],
    ["a third click returns to the default", { sort: "last_seen", direction: "asc" }, "last_seen", null],
    ["another column starts over at its largest", { sort: "last_seen", direction: "asc" }, "page_views", { sort: "page_views", direction: "desc" }],
    ["the default column flips first", null, "visits", { sort: "visits", direction: "asc" }],
    ["the flipped default returns to the default", { sort: "visits", direction: "asc" }, "visits", null],
    ["choosing the default column's own order is the default", { sort: "last_seen", direction: "desc" }, "visits", null],
  ] as const)("%s", (_, current, clicked, want) => {
    expect(nextOrder<Key>(current, clicked, DEFAULT)).toEqual(want);
  });
});

describe("effectiveOrder", () => {
  it("names the order the rows are in when none was chosen", () => {
    expect(effectiveOrder<Key>(null, DEFAULT)).toEqual({ sort: "visits", direction: "desc" });
  });

  it("names the chosen order when there is one", () => {
    expect(effectiveOrder<Key>({ sort: "page_views", direction: "asc" }, DEFAULT)).toEqual({
      sort: "page_views",
      direction: "asc",
    });
  });
});
