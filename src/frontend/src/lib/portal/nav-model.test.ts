import { describe, expect, it } from "vitest";

import {
  MANAGE_ITEMS,
  manageItemsFor,
  partitionByReadiness,
  resolveZoneItem,
  ZONE_DEFAULT_ITEM,
  zoneById,
  zoneItems,
} from "./nav-model";

const defaults = Object.entries(ZONE_DEFAULT_ITEM);

describe("zone item defaults", () => {
  it("name a zone the rail has", () => {
    for (const [zone] of defaults) expect(zoneById(zone), zone).toBeDefined();
  });

  it("name an item the pane always renders", () => {
    // A default the pane can filter out (planned / unbuilt) would highlight a
    // row that isn't there.
    for (const [zone, id] of defaults) {
      const { live } = partitionByReadiness(zoneItems(zone), false);
      expect(live.map((i) => i.id), zone).toContain(id);
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

  it("stays null for a zone whose no-item view is no menu row", () => {
    expect(resolveZoneItem("manage", null)).toBeNull();
    expect(resolveZoneItem("manage", "trend")).toBeNull();
    expect(resolveZoneItem("manage", "data-health")).toBe("data-health");
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
