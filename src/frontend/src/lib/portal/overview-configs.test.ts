import { describe, expect, it } from "vitest";

import { GROUPS } from "@/lib/insight/groups";
import { ZONE_SECTIONS } from "@/lib/portal/nav-model";
import { sectionMetricKeys } from "@/lib/portal/lens-configs";
import { DEFAULT_OVERVIEW_ITEM, OVERVIEW_ITEMS, overviewMetricKeys } from "./overview-configs";

const KNOWN_KEYS = new Set(
  GROUPS.flatMap((g) => g.collection.metrics.map((m) => m.key)),
);
const NAV_ITEM_IDS = (ZONE_SECTIONS.overview ?? []).flatMap((group) =>
  group.items.map((i) => i.id),
);

describe("OVERVIEW_ITEMS registry", () => {
  it("covers every Overview pane item declared in nav-model", () => {
    for (const id of NAV_ITEM_IDS) {
      expect(OVERVIEW_ITEMS[id], id).toBeDefined();
    }
  });
  it("has no orphan items missing from nav-model", () => {
    for (const id of Object.keys(OVERVIEW_ITEMS)) {
      expect(NAV_ITEM_IDS.includes(id), id).toBe(true);
    }
  });
  it("references only metric keys that exist, per item and in total", () => {
    for (const [id, config] of Object.entries(OVERVIEW_ITEMS)) {
      const keys = sectionMetricKeys(config);
      expect(keys.length, id).toBeLessThanOrEqual(50);
      for (const k of keys) expect(KNOWN_KEYS.has(k), `${id}: ${k}`).toBe(true);
    }
    expect(overviewMetricKeys().length).toBeLessThanOrEqual(50);
  });
  it("spans multiple metric families in the union (zone-stable grid)", () => {
    const union = overviewMetricKeys();
    expect(union.some((k) => k.startsWith("git."))).toBe(true);
    expect(union.some((k) => k.startsWith("collab."))).toBe(true);
    expect(union.some((k) => k.startsWith("ai."))).toBe(true);
  });
  it("gives every item at least one section", () => {
    for (const [id, config] of Object.entries(OVERVIEW_ITEMS)) {
      expect(config.sections.length, id).toBeGreaterThanOrEqual(1);
    }
  });

  it("keeps the router default item present", () => {
    expect(OVERVIEW_ITEMS[DEFAULT_OVERVIEW_ITEM]).toBeDefined();
  });

  it("the at-a-glance headline spans code, delivery, comms and cost", () => {
    // The headline used to be git + comms + cost only, so an org whose people
    // work in tickets rather than repos read as idle at the top level — and the
    // gap was invisible on a dataset where every git tile happened to have data.
    const headline = OVERVIEW_ITEMS[DEFAULT_OVERVIEW_ITEM]!.sections.find(
      (s) => s.kind === "headline",
    );
    const metrics = headline && "metrics" in headline ? headline.metrics : [];
    for (const family of ["git.", "tasks.", "collab.", "ai."]) {
      expect(
        metrics.some((m) => m.startsWith(family)),
        `headline is missing the ${family} family`,
      ).toBe(true);
    }
  });

  it("direction-cards keys are variant-independent", () => {
    const compact = sectionMetricKeys({
      title: "t",
      sections: [{ kind: "direction-cards", variant: "compact" }],
    });
    const full = sectionMetricKeys({
      title: "t",
      sections: [{ kind: "direction-cards", variant: "full" }],
    });
    expect([...compact].sort()).toEqual([...full].sort());
  });
});
