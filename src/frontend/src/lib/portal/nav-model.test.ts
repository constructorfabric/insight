import { describe, expect, it } from "vitest";

import {
  defaultZoneItem,
  MANAGE_ITEMS,
  manageItemsFor,
  partitionByReadiness,
  resolveZoneItem,
  ZONES,
  zoneItems,
} from "./nav-model";

const defaults = ZONES.map((z) => [z.id, defaultZoneItem(z.id)] as const);

describe("zone item defaults", () => {
  it("open each zone on its first built entry", () => {
    expect(Object.fromEntries(defaults)).toEqual({
      overview: "at-a-glance",
      directions: null,
      person: null,
      people: "roster",
      aicost: "overview",
      scorecard: null,
      reports: "report-builder",
      manage: "metric-catalog",
    });
  });

  it("name an item the pane always renders", () => {
    for (const [zone, id] of defaults) {
      if (id == null) continue;
      const { live } = partitionByReadiness(zoneItems(zone), false);
      expect(live.map((i) => i.id), zone).toContain(id);
    }
  });

  it("are null only where the zone lists nothing built", () => {
    for (const [zone, id] of defaults) {
      const { live } = partitionByReadiness(zoneItems(zone), false);
      expect(id === null, zone).toBe(live.length === 0);
    }
  });

  it("name an item every viewer can see, not an admin-only one", () => {
    for (const [zone, id] of defaults) {
      expect(zoneItems(zone).find((i) => i.id === id)?.adminOnly, zone).toBeFalsy();
    }
  });
});

describe("resolveZoneItem", () => {
  it("keeps an item the zone lists", () => {
    expect(resolveZoneItem("overview", "trend")).toBe("trend");
  });

  it("falls back for an item that belongs to another zone", () => {
    expect(resolveZoneItem("people", "trend")).toBe("roster");
  });

  it("falls back when the URL names none", () => {
    expect(resolveZoneItem("aicost", null)).toBe("overview");
  });

  it("keeps a Manage item the URL names, and falls back for one it does not", () => {
    expect(resolveZoneItem("manage", "data-health")).toBe("data-health");
    expect(resolveZoneItem("manage", "trend")).toBe("metric-catalog");
  });

  it("stays null for a zone with nothing built to open on", () => {
    expect(resolveZoneItem("scorecard", null)).toBeNull();
    expect(resolveZoneItem("person", null)).toBeNull();
  });
});

/**
 * The Manage pane per viewer: admin-only surfaces exist for admins alone.
 * The non-admin list must stay a strict subset — dropping a shared item or
 * reordering would silently reshape the pane every operator already knows.
 */
describe("manageItemsFor", () => {
  it("gives an admin the full pane", () => {
    expect(manageItemsFor(true)).toEqual(MANAGE_ITEMS);
  });

  it("drops exactly the admin-only surfaces for everyone else", () => {
    const visible = manageItemsFor(false);

    expect(visible.map((i) => i.id)).not.toContain("identities");
    expect(visible).toEqual(MANAGE_ITEMS.filter((i) => !i.adminOnly));
  });
});
