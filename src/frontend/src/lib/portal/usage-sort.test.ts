import { describe, expect, it } from "vitest";

import { effectiveOrder, nextOrder } from "./usage-sort";

type Key = "visits" | "page_views" | "last_seen";

const DEFAULT: Key = "visits";

describe("nextOrder", () => {
  it.each([
    ["a column starts at its largest", null, "last_seen", { sort: "last_seen", direction: "desc" }],
    ["the same column flips", { sort: "last_seen", direction: "desc" }, "last_seen", { sort: "last_seen", direction: "asc" }],
    ["a third click returns to the default", { sort: "last_seen", direction: "asc" }, "last_seen", null],
    ["another column starts over at its largest", { sort: "last_seen", direction: "asc" }, "page_views", { sort: "page_views", direction: "desc" }],
    ["the default column starts at its largest like any other", null, "visits", { sort: "visits", direction: "desc" }],
    ["the default column flips on its second click", { sort: "visits", direction: "desc" }, "visits", { sort: "visits", direction: "asc" }],
    ["the default column resets on its third click", { sort: "visits", direction: "asc" }, "visits", null],
    ["leaving a column for the default one starts the default at its largest", { sort: "last_seen", direction: "desc" }, "visits", { sort: "visits", direction: "desc" }],
  ] as const)("%s", (_, current, clicked, want) => {
    expect(nextOrder<Key>(current, clicked)).toEqual(want);
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
