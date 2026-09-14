import { describe, expect, it } from "vitest";

import { dashboardItems } from "./dashboard-items";

describe("dashboardItems", () => {
  it("draws the item list a board carries, in order", () => {
    expect(
      dashboardItems({
        title: "Engineering",
        items: [
          { heading: "Per person" },
          { widget: "commits_bar" },
          { text: "Bots excluded." },
        ],
      })
    ).toEqual([
      { heading: "Per person" },
      { widget: "commits_bar" },
      { text: "Bots excluded." },
    ]);
  });

  it("reads the older shorthand as a list of nothing but widgets", () => {
    expect(
      dashboardItems({ title: "Engineering", widgets: ["one", "two"] })
    ).toEqual([{ widget: "one" }, { widget: "two" }]);
  });

  it("prefers the item list when a board somehow carries both", () => {
    expect(
      dashboardItems({
        title: "Engineering",
        items: [{ widget: "new" }],
        widgets: ["old"],
      })
    ).toEqual([{ widget: "new" }]);
  });

  it("draws nothing for a board that says nothing", () => {
    expect(dashboardItems({ title: "Empty" })).toEqual([]);
  });
});
