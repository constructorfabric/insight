import { describe, expect, it } from "vitest";

import { ZONES } from "./nav-model";
import { parseNavPolicy } from "./nav-policy";
import { railItemFor, visibleRailItems } from "./rail-model";

const ids = (items: readonly { id: string }[]) => items.map((item) => item.id);
const zones = (...wanted: string[]) => ZONES.filter((zone) => wanted.includes(zone.id));

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
    expect(ids(visibleRailItems(zones("aicost")))).toEqual(["explore"]);
  });

  it("drops an item none of whose zones is visible", () => {
    expect(ids(visibleRailItems(zones("person", "manage")))).toEqual([
      "people",
      "manage",
    ]);
  });

  it("orders the rail Home, Dashboards, Explore, People, Reports, Manage", () => {
    expect(ids(visibleRailItems(ZONES))).toEqual([
      "home",
      "dashboards",
      "explore",
      "people",
      "reports",
      "manage",
    ]);
  });

  it("mutes an item whose zones are all planned", () => {
    const planned = (items: ReturnType<typeof visibleRailItems>) =>
      Object.fromEntries(items.map((item) => [item.id, item.planned]));

    expect(
      planned(
        visibleRailItems(
          zones("overview", "directions", "aicost"),
          parseNavPolicy({ planned: ["zone:overview", "zone:aicost"] }),
        ),
      ),
    ).toEqual({ home: true, explore: false });
  });
});
