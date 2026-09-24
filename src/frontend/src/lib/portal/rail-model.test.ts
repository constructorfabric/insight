import { describe, expect, it } from "vitest";

import { railItemFor, visibleRailItems } from "./rail-model";

const ids = (items: readonly { id: string }[]) => items.map((item) => item.id);

describe("railItemFor", () => {
  it.each([
    ["overview", "home"],
    ["custom", "dashboards"],
    ["directions", "explore"],
    ["aicost", "explore"],
    ["person", "people"],
    ["people", "people"],
    ["reports", "reports"],
    ["manage", "manage"],
  ])("puts the %s zone under %s", (zone, rail) => {
    expect(railItemFor(zone)?.id).toBe(rail);
  });

  it("has no rail item for the retired Scorecard zone", () => {
    expect(railItemFor("scorecard")).toBeUndefined();
  });
});

describe("visibleRailItems", () => {
  it("offers an item when any one of its zones is visible", () => {
    expect(ids(visibleRailItems(["aicost"], false))).toEqual(["explore"]);
  });

  it("drops an item none of whose zones is visible", () => {
    expect(ids(visibleRailItems(["person", "manage"], false))).toEqual([
      "people",
      "manage",
    ]);
  });

  it("orders the rail Home, Dashboards, Explore, People, Reports, Manage", () => {
    const every = [
      "manage",
      "reports",
      "people",
      "person",
      "aicost",
      "directions",
      "custom",
      "overview",
    ];

    expect(ids(visibleRailItems(every, true))).toEqual([
      "home",
      "dashboards",
      "explore",
      "people",
      "reports",
      "manage",
    ]);
  });

  it("keeps Reports off the rail until planned sections are shown", () => {
    expect(ids(visibleRailItems(["overview", "reports"], false))).toEqual(["home"]);
    expect(ids(visibleRailItems(["overview", "reports"], true))).toEqual([
      "home",
      "reports",
    ]);
  });
});
