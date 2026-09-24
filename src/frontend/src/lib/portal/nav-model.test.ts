import { describe, expect, it } from "vitest";

import {
  defaultZoneItem,
  MANAGE_ITEMS,
  manageGroupsFor,
  orderByReadiness,
  partitionByReadiness,
  peopleItemsFor,
  resolveZoneItem,
  ZONES,
  zoneItems,
} from "./nav-model";

const defaults = ZONES.map((z) => [z.id, defaultZoneItem(z.id)] as const);

const openable = (zone: string) =>
  partitionByReadiness(
    zoneItems(zone).filter((i) => !i.unbuilt),
    false,
  ).live;

describe("zone item defaults", () => {
  it("open each zone on its first catalog entry", () => {
    expect(Object.fromEntries(defaults)).toEqual({
      overview: "at-a-glance",
      directions: null,
      person: null,
      people: "roster",
      aicost: "overview",
      reports: null,
      custom: null,
      manage: "exclusions",
    });
  });

  it("name an item the pane always renders", () => {
    for (const [zone, id] of defaults) {
      if (id == null) continue;
      expect(openable(zone).map((i) => i.id), zone).toContain(id);
    }
  });

  it("are null only where the zone lists nothing to open", () => {
    for (const [zone, id] of defaults) {
      expect(id === null, zone).toBe(openable(zone).length === 0);
    }
  });

  it("name an item every viewer can see, not an admin-only one", () => {
    for (const [zone, id] of defaults) {
      expect(zoneItems(zone).find((i) => i.id === id)?.adminOnly, zone).toBeFalsy();
    }
  });
});

describe("readiness", () => {
  const rows = [
    { id: "live" },
    { id: "unbuilt", unbuilt: true },
    { id: "install-planned", readiness: "planned" as const },
  ];
  const ids = (items: readonly { id: string }[]) => items.map((i) => i.id);

  it("keeps an unbuilt row in place, and only while planned sections are shown", () => {
    const shown = partitionByReadiness(rows, true);
    const hidden = partitionByReadiness(rows, false);

    expect(ids(shown.live)).toEqual(["live", "unbuilt"]);
    expect(ids(shown.planned)).toEqual(["install-planned"]);
    expect(ids(hidden.live)).toEqual(["live"]);
    expect(ids(hidden.planned)).toEqual([]);
  });

  it("orders live rows before install-planned ones", () => {
    expect(ids(orderByReadiness(rows, true))).toEqual([
      "live",
      "unbuilt",
      "install-planned",
    ]);
    expect(ids(orderByReadiness(rows, false))).toEqual(["live"]);
  });
});

describe("the retired zones", () => {
  it("no longer lists Scorecard", () => {
    expect(ZONES.map((z) => z.id)).not.toContain("scorecard");
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
    expect(resolveZoneItem("manage", "trend")).toBe("exclusions");
  });

  it("stays null for a zone with no catalog items", () => {
    expect(resolveZoneItem("person", null)).toBeNull();
  });

  it("never opens a row that has nothing behind it yet", () => {
    expect(resolveZoneItem("reports", "snapshots")).toBeNull();
    expect(resolveZoneItem("manage", "access")).toBe("exclusions");
  });
});

describe("the ingestion lens", () => {
  it("is admin-only: bronze rows carry no tenant to scope it by", () => {
    const item = MANAGE_ITEMS.find((i) => i.id === "ingestion");
    expect(item?.adminOnly).toBe(true);
    const shows = (isAdmin: boolean) =>
      manageGroupsFor({ isAdmin, canManagePreviews: false })
        .flatMap((g) => g.items)
        .some((i) => i.id === "ingestion");
    expect(shows(false)).toBe(false);
    expect(shows(true)).toBe(true);
  });

  it("is its own lens, beside connector health rather than inside it", () => {
    const ids = MANAGE_ITEMS.map((i) => i.id);
    expect(ids).toContain("connector-health");
    expect(ids).toContain("ingestion");
    expect(ids.indexOf("ingestion")).not.toBe(ids.indexOf("connector-health"));
  });
});

describe("manageGroupsFor", () => {
  const labelsOf = (isAdmin: boolean, canManagePreviews = false) =>
    Object.fromEntries(
      manageGroupsFor({ isAdmin, canManagePreviews }).map((g) => [
        g.label,
        g.items.map((i) => i.label),
      ]),
    );

  it("groups an admin's pane into Data, People & access and Platform", () => {
    expect(labelsOf(true, true)).toEqual({
      Data: ["Sources & connectors", "Data exclusions", "Org snapshots"],
      "People & access": [
        "Identities · roles & taxonomy",
        "Access",
        "Group management",
      ],
      Platform: [
        "Ingestion",
        "Previews",
        "AI assistant config",
        "What's new",
        "Platform usage",
        "MCP servers",
        "Config & setup",
        "Scorecard management",
      ],
    });
  });

  it("drops exactly the gated surfaces for everyone else", () => {
    expect(labelsOf(false)).toEqual({
      Data: ["Data exclusions", "Org snapshots"],
      "People & access": ["Access", "Group management"],
      Platform: [
        "AI assistant config",
        "What's new",
        "MCP servers",
        "Config & setup",
        "Scorecard management",
      ],
    });
  });

  it("gates previews independently of admin-ness", () => {
    const ids = manageGroupsFor({ isAdmin: false, canManagePreviews: true })
      .flatMap((g) => g.items)
      .map((i) => i.id);

    expect(ids).toContain("previews");
    expect(ids).not.toContain("identities");
  });

  it("marks Access as a row with nothing behind it yet", () => {
    const access = MANAGE_ITEMS.find((i) => i.id === "access");
    expect(access?.unbuilt).toBe(true);
  });
});

describe("peopleItemsFor", () => {
  it("names the team views for an organisation with reporting lines", () => {
    const items = peopleItemsFor(false);

    expect(items.map((item) => item.id)).toEqual(["roster", "employees"]);
    expect(items.map((item) => item.label)).toEqual(["My team", "Roster · by role"]);
  });

  it("names the same views for an organisation with no reporting lines", () => {
    const items = peopleItemsFor(true);

    expect(items.map((item) => item.id)).toEqual(["roster", "employees"]);
    expect(items.map((item) => item.label)).toEqual(["Overview", "Roster"]);
  });
});
