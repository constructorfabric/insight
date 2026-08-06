import { describe, expect, it } from "vitest";

import { DIRECTION_LENSES, lensEntry } from "./lens-configs";
import {
  MANAGE_ITEMS,
  PEOPLE_ITEMS,
  partitionByReadiness,
  ZONES,
  ZONE_SECTIONS,
} from "./nav-model";

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

  it("always lists product-side gaps, demoted — they are roadmap, not noise", () => {
    const { planned } = partitionByReadiness(entries, false);
    expect(planned.map((e) => e.id)).toEqual(["planned"]);
  });

  it("hides our own unbuilt screens until the viewer opts in", () => {
    expect(partitionByReadiness(entries, false).planned.map((e) => e.id)).not.toContain(
      "unbuilt",
    );
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

describe("nav classification invariants", () => {
  it("hiding planned work still leaves every zone the portal can render", () => {
    const { live } = partitionByReadiness(ZONES, false);
    // The five zones with real views must survive the strictest filter.
    expect(live.map((z) => z.id)).toEqual([
      "overview",
      "directions",
      "person",
      "people",
      "aicost",
      "manage",
    ]);
  });

  it("Manage keeps exactly the two surfaces that read live data", () => {
    const { live } = partitionByReadiness(MANAGE_ITEMS, false);
    expect(live.map((i) => i.id)).toEqual(["metric-catalog", "data-health"]);
  });

  it("every Overview item is live — that zone has no placeholders", () => {
    const items = (ZONE_SECTIONS.overview ?? []).flatMap((g) => g.items);
    expect(items.every((i) => i.readiness == null)).toBe(true);
  });

  it("People marks the cohort-pipeline item as planned, not unbuilt", () => {
    const median = PEOPLE_ITEMS.find((i) => i.id === "median-by-role");
    expect(median?.readiness).toBe("planned");
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
        expect(e.comingSoon, `${dir}/${lens}`).toMatch(/semantic layer/i);
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

  it("Sales and Support wait on the product, so they stay listed", () => {
    for (const dir of ["sales", "support"]) {
      for (const lens of Object.keys(DIRECTION_LENSES[dir]!)) {
        expect((lensEntry(dir, lens) as { readiness: string }).readiness, dir).toBe(
          "planned",
        );
      }
    }
  });
});
