import { describe, expect, it } from "vitest";

import {
  DIRECTION_LENSES,
  lensEntry,
  visibleDirections,
  visibleLenses,
} from "./lens-configs";
import {
  DIRECTIONS,
  MANAGE_ITEMS,
  PEOPLE_ITEMS,
  partitionByReadiness,
  ZONES,
  ZONE_SECTIONS,
  zoneSections,
} from "./nav-model";
import { parseNavPolicy } from "./nav-policy";

describe("partitionByReadiness", () => {
  const entries = [
    { id: "live" },
    { id: "planned", readiness: "planned" as const },
  ];

  it("keeps unmarked entries live", () => {
    for (const show of [true, false]) {
      const { live } = partitionByReadiness(entries, show);
      expect(live.map((entry) => entry.id)).toEqual(["live"]);
    }
  });

  it("shows marked entries only when planned sections are enabled", () => {
    expect(partitionByReadiness(entries, false).planned).toEqual([]);
    expect(
      partitionByReadiness(entries, true).planned.map((entry) => entry.id)
    ).toEqual(["planned"]);
  });
});

describe("configuration-owned planning", () => {
  it("keeps static zone and item catalogs free of planning values", () => {
    const items = [
      ...MANAGE_ITEMS,
      ...PEOPLE_ITEMS,
      ...Object.values(ZONE_SECTIONS).flatMap((groups) =>
        groups.flatMap((group) => group.items)
      ),
    ];

    expect(ZONES.every((zone) => !("readiness" in zone))).toBe(true);
    expect(items.every((item) => item.readiness == null)).toBe(true);
  });

  it("keeps roadmap notes free of planning values", () => {
    for (const [direction, lenses] of Object.entries(DIRECTION_LENSES)) {
      for (const lens of Object.keys(lenses)) {
        const entry = lensEntry(direction, lens)!;
        if (!("comingSoon" in entry)) continue;

        expect("readiness" in entry, `${direction}/${lens}`).toBe(false);
      }
    }
  });

  it("shows every direction and lens when the policy is empty", () => {
    const policy = parseNavPolicy({ hide: [], planned: [] });

    expect(visibleDirections(false, policy).map((direction) => direction.id)).toEqual(
      DIRECTIONS.map((direction) => direction.id)
    );
    for (const direction of DIRECTIONS) {
      expect(visibleLenses(direction, false, policy)).toEqual(direction.lenses);
    }
  });

  it("marks only paths named by the install policy", () => {
    const policy = parseNavPolicy({
      planned: ["zone:aicost/item:per-tool"],
    });
    const items = zoneSections("aicost", policy).flatMap(
      (group) => group.items
    );

    expect(items.find((item) => item.id === "per-tool")?.readiness).toBe(
      "planned"
    );
    expect(items.find((item) => item.id === "autofix")?.readiness).toBeUndefined();
  });
});
