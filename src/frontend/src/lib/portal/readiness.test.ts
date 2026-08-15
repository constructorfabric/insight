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
  zoneItems,
} from "./nav-model";
import { PANE_ITEM_COMING_SOON } from "./aicost-configs";

describe("partitionByReadiness", () => {
  const entries = [
    { id: "live" },
    { id: "planned", readiness: "planned" as const },
    { id: "unbuilt", readiness: "unbuilt" as const },
  ];

  it("keeps unmarked entries live and never demotes them", () => {
    for (const show of [true, false]) {
      const { live } = partitionByReadiness(entries, show);
      expect(live.map((e) => e.id)).toEqual(["live"]);
    }
  });

  it("hides both readiness tiers when the reader opts out", () => {
    const { live, planned } = partitionByReadiness(entries, false);
    expect(planned).toEqual([]);
    expect(live.map((e) => e.id)).toEqual(["live"]);
  });

  it("lists both tiers, demoted, when the reader opts in", () => {
    expect(partitionByReadiness(entries, true).planned.map((e) => e.id)).toEqual([
      "planned",
      "unbuilt",
    ]);
  });

  it("returns everything exactly once — nothing is dropped silently", () => {
    const { live, planned } = partitionByReadiness(entries, true);
    expect([...live, ...planned]).toHaveLength(entries.length);
  });
});

describe("navigation with planned sections hidden", () => {
  it("leaves People only the entries that render a view", () => {
    const { live } = partitionByReadiness(PEOPLE_ITEMS, false);
    expect(live.map((i) => i.id)).toEqual(["roster", "employees"]);
  });

  it("leaves AI & Cost only the entries that render a view", () => {
    const items = (ZONE_SECTIONS.aicost ?? []).flatMap((g) => g.items);
    const { live } = partitionByReadiness(items, false);
    expect(live.map((i) => i.id)).toEqual([
      "overview",
      "adoption-funnel",
      "by-unit-role",
    ]);
  });

  it("drops a direction whose every lens is filtered out", () => {
    expect(visibleDirections(false).map((d) => d.id)).toEqual([
      "dev",
      "collab",
      "wiki",
    ]);
  });

  it("keeps every direction once the reader opts in", () => {
    expect(visibleDirections(true).map((d) => d.id)).toEqual(
      DIRECTIONS.map((d) => d.id),
    );
  });

  it("leaves Development only the lenses that render sections", () => {
    const dev = DIRECTIONS.find((d) => d.id === "dev")!;
    expect(visibleLenses(dev, false)).toEqual([
      "Overview",
      "Git output",
      "Delivery",
      "Flow",
    ]);
  });

  it("hides no zone, item or lens that renders something", () => {
    for (const zone of ZONES) {
      if (zone.readiness != null) continue;
      for (const item of zoneItems(zone.id)) {
        if (item.readiness != null) continue;
        expect(
          partitionByReadiness(zoneItems(zone.id), false).live,
          `${zone.id}/${item.id}`,
        ).toContainEqual(item);
      }
    }
  });
});

describe("nav classification invariants", () => {
  it("hiding planned work still leaves every zone the portal can render", () => {
    const { live } = partitionByReadiness(ZONES, false);
    // Every zone with a real view must survive the strictest filter.
    expect(live.map((z) => z.id)).toEqual([
      "overview",
      "directions",
      "person",
      "people",
      "aicost",
      "reports",
      "manage",
    ]);
  });

  it("Manage keeps exactly the surfaces that render something", () => {
    const { live } = partitionByReadiness(MANAGE_ITEMS, false);
    expect(live.map((i) => i.id)).toEqual([
      "metric-catalog",
      "identities",
      "data-health",
      "whats-new",
    ]);
  });

  it("every Overview item is live — that zone has no placeholders", () => {
    const items = (ZONE_SECTIONS.overview ?? []).flatMap((g) => g.items);
    expect(items.every((i) => i.readiness == null)).toBe(true);
  });

  it("People marks the cohort view as ours to build", () => {
    const median = PEOPLE_ITEMS.find((i) => i.id === "median-by-role");
    expect(median?.readiness).toBe("unbuilt");
  });

  it("no zone or item is marked with an unknown readiness value", () => {
    const all = [
      ...ZONES,
      ...MANAGE_ITEMS,
      ...PEOPLE_ITEMS,
      ...Object.values(ZONE_SECTIONS).flatMap((groups) =>
        groups.flatMap((g) => g.items),
      ),
    ];
    for (const e of all) {
      if (e.readiness != null) {
        expect(["planned", "unbuilt"]).toContain(e.readiness);
      }
    }
  });
});

describe("lens roadmap entries carry a reason", () => {
  const roadmap = Object.entries(DIRECTION_LENSES).flatMap(([dir, lenses]) =>
    Object.keys(lenses)
      .map((lens) => ({ dir, lens, entry: lensEntry(dir, lens)! }))
      .filter((x) => "comingSoon" in x.entry),
  );

  it("finds roadmap lenses to check", () => expect(roadmap.length).toBeGreaterThan(5));

  it("every roadmap lens declares whether it waits on the product or on us", () => {
    for (const { dir, lens, entry } of roadmap) {
      expect(["planned", "unbuilt"], `${dir}/${lens}`).toContain(
        (entry as { readiness: string }).readiness,
      );
    }
  });

  it("wording matches the reason — a product gap never reads as in-development", () => {
    for (const { dir, lens, entry } of roadmap) {
      const e = entry as { comingSoon: string; readiness: string };
      if (e.readiness === "planned") {
        expect(e.comingSoon, `${dir}/${lens}`).toMatch(/not available yet/i);
        expect(e.comingSoon, `${dir}/${lens}`).not.toMatch(/in development/i);
      } else {
        expect(e.comingSoon, `${dir}/${lens}`).toMatch(/in development/i);
      }
    }
  });

  it("Repositories and Elements are ours to build, not a data request", () => {
    for (const lens of ["Repositories", "Elements"]) {
      const entry = lensEntry("dev", lens) as { readiness: string };
      expect(entry.readiness, lens).toBe("unbuilt");
    }
  });

  it("Sales and Support wait on the product, so they carry the product-gap tag", () => {
    for (const dir of ["sales", "support"]) {
      for (const lens of Object.keys(DIRECTION_LENSES[dir]!)) {
        expect((lensEntry(dir, lens) as { readiness: string }).readiness, dir).toBe(
          "planned",
        );
      }
    }
  });
});

describe("pane item roadmap entries carry a reason", () => {
  const tagged = Object.entries(PANE_ITEM_COMING_SOON).map(([id, copy]) => ({
    id,
    copy,
    readiness: zoneItems("aicost").find((i) => i.id === id)?.readiness,
  }));

  it("finds pane items to check", () => expect(tagged.length).toBeGreaterThan(5));

  it("every one declares whether it waits on the product or on us", () => {
    for (const { id, readiness } of tagged) {
      expect(["planned", "unbuilt"], id).toContain(readiness);
    }
  });

  it("a product gap never claims the screen is missing", () => {
    for (const { id, copy, readiness } of tagged) {
      if (readiness !== "planned") continue;
      expect(copy, id).not.toMatch(/not built yet|is pending|in development/i);
    }
  });

  it("a screen we owe never claims the data is absent", () => {
    for (const { id, copy, readiness } of tagged) {
      if (readiness !== "unbuilt") continue;
      expect(copy, id).not.toMatch(/no .* data is collected|data is not available yet/i);
    }
  });
});
