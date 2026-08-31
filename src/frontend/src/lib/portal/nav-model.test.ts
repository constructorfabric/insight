import { describe, expect, it } from "vitest";

import {
  defaultZoneItem,
  MANAGE_ITEMS,
  manageItemsFor,
  partitionByReadiness,
  peopleItemsFor,
  resolveZoneItem,
  ZONES,
  zoneItems,
} from "./nav-model";

const defaults = ZONES.map((z) => [z.id, defaultZoneItem(z.id)] as const);

describe("zone item defaults", () => {
  it("open each zone on its first catalog entry", () => {
    expect(Object.fromEntries(defaults)).toEqual({
      overview: "at-a-glance",
      directions: null,
      person: null,
      people: "roster",
      aicost: "overview",
      scorecard: "fixed",
      reports: "delivery-trend",
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

  it("are null only where the zone lists no items", () => {
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
    expect(resolveZoneItem("manage", "connector-health")).toBe("connector-health");
    expect(resolveZoneItem("manage", "trend")).toBe("metric-catalog");
  });

  it("stays null for a zone with no catalog items", () => {
    expect(resolveZoneItem("person", null)).toBeNull();
  });
});

/**
 * The Manage pane per viewer: admin-only surfaces exist for admins alone.
 * The non-admin list must stay a strict subset — dropping a shared item or
 * reordering would silently reshape the pane every operator already knows.
 */
describe("manageItemsFor", () => {
  it("gives a viewer passing every gate the full pane", () => {
    expect(
      manageItemsFor({ isAdmin: true, canManagePreviews: true }),
    ).toEqual(MANAGE_ITEMS);
  });

  it("drops exactly the gated surfaces for everyone else", () => {
    const visible = manageItemsFor({
      isAdmin: false,
      canManagePreviews: false,
    });

    expect(visible.map((i) => i.id)).not.toContain("identities");
    expect(visible.map((i) => i.id)).not.toContain("previews");
    expect(visible).toEqual(
      MANAGE_ITEMS.filter((i) => !i.adminOnly && !i.previewsGated),
    );
  });

  it("gates previews independently of admin-ness", () => {
    const previewsOnly = manageItemsFor({
      isAdmin: false,
      canManagePreviews: true,
    });

    expect(previewsOnly.map((i) => i.id)).toContain("previews");
    expect(previewsOnly.map((i) => i.id)).not.toContain("identities");
  });
});

describe("peopleItemsFor", () => {
  it("keeps the reporting-line names when there is a reporting line", () => {
    const labels = peopleItemsFor(false).map((item) => item.label);

    expect(labels).toContain("Employees");
    expect(labels).toContain("Median by Role");
  });

  it("names the same views for an organisation with no reporting lines", () => {
    // Same ids, because the pane routes on them.
    const items = peopleItemsFor(true);

    expect(items.map((item) => item.id)).toEqual(["roster", "employees"]);
    expect(items.map((item) => item.label)).toEqual(["Overview", "Roster"]);
  });

  it("drops the by-role cut a flat roster cannot make", () => {
    // No job titles in that roster, so the median has nothing to group by.
    expect(peopleItemsFor(true).map((item) => item.id)).not.toContain(
      "median-by-role",
    );
  });
});
