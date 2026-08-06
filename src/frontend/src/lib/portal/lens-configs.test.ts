import { describe, expect, it } from "vitest";

import { GROUPS } from "@/lib/insight/groups";
import { DIRECTIONS } from "@/lib/portal/nav-model";
import {
  DIRECTION_LENSES,
  directionMetricKeys,
  lensEntry,
  sectionMetricKeys,
  type SectionSpec,
} from "./lens-configs";

const KNOWN_KEYS = new Set(
  GROUPS.flatMap((g) => g.collection.metrics.map((m) => m.key)),
);

describe("DIRECTION_LENSES registry", () => {
  it("covers every direction and lens declared in nav-model", () => {
    for (const d of DIRECTIONS) {
      for (const lens of d.lenses) {
        expect(lensEntry(d.id, lens), `${d.id}/${lens}`).toBeDefined();
      }
    }
  });

  it("references only metric keys that exist in the groups registry", () => {
    for (const [dir, lenses] of Object.entries(DIRECTION_LENSES)) {
      for (const [lens, entry] of Object.entries(lenses)) {
        if ("comingSoon" in entry) continue;
        for (const key of sectionMetricKeys(entry)) {
          expect(KNOWN_KEYS.has(key), `${dir}/${lens}: ${key}`).toBe(true);
        }
      }
    }
  });

  it("stays under the API metric cap per lens", () => {
    for (const lenses of Object.values(DIRECTION_LENSES)) {
      for (const entry of Object.values(lenses)) {
        if ("comingSoon" in entry) continue;
        expect(sectionMetricKeys(entry).length).toBeLessThanOrEqual(50);
      }
    }
  });

  it("has no orphan configs — every registry dir/lens exists in nav-model", () => {
    const navLenses: Record<string, Set<string>> = {};
    for (const d of DIRECTIONS) navLenses[d.id] = new Set(d.lenses);
    for (const [dir, lenses] of Object.entries(DIRECTION_LENSES)) {
      for (const lens of Object.keys(lenses)) {
        expect(navLenses[dir]?.has(lens), `${dir}/${lens}`).toBe(true);
      }
    }
  });

  it("gives every non-comingSoon entry at least one section", () => {
    for (const [dir, lenses] of Object.entries(DIRECTION_LENSES)) {
      for (const [lens, entry] of Object.entries(lenses)) {
        if ("comingSoon" in entry) continue;
        expect(entry.sections.length, `${dir}/${lens}`).toBeGreaterThanOrEqual(1);
      }
    }
  });

  it("never has two composition sections sharing the same metric (compData is keyed by metric)", () => {
    for (const [dir, lenses] of Object.entries(DIRECTION_LENSES)) {
      for (const [lens, entry] of Object.entries(lenses)) {
        if ("comingSoon" in entry) continue;
        const compMetrics = entry.sections
          .filter(
            (s): s is Extract<SectionSpec, { kind: "composition" }> => s.kind === "composition",
          )
          .map((s) => s.metric);
        expect(new Set(compMetrics).size, `${dir}/${lens}`).toBe(compMetrics.length);
      }
    }
  });
});

describe("directionMetricKeys", () => {
  it("stays under the API metric cap per direction — the union must stay requestable in one grid", () => {
    for (const dir of Object.keys(DIRECTION_LENSES)) {
      expect(directionMetricKeys(dir).length, dir).toBeLessThanOrEqual(50);
    }
  });

  it("spans every lens of the direction, not just one (dev has both git.* and tasks.*)", () => {
    const keys = directionMetricKeys("dev");
    expect(keys.some((k) => k.startsWith("git."))).toBe(true);
    expect(keys.some((k) => k.startsWith("tasks."))).toBe(true);
  });
});

describe("sectionMetricKeys — Overview section kinds", () => {
  it("collects attention metrics", () => {
    const keys = sectionMetricKeys({
      title: "t",
      sections: [{ kind: "attention", metrics: ["git.commits", "wiki.edits"], max: 8 }],
    });
    expect(keys.sort()).toEqual(["git.commits", "wiki.edits"]);
  });
  it("derives direction-cards keys from every configured Overview lens headline", () => {
    const keys = sectionMetricKeys({
      title: "t",
      sections: [{ kind: "direction-cards", variant: "full" }],
    });
    expect(keys).toContain("git.commits");
    expect(keys).toContain("collab.messages_sent");
    expect(keys).toContain("wiki.pages_created");
  });
  it("derives coverage-radar keys from every group's card preview", () => {
    const keys = new Set(
      sectionMetricKeys({ title: "t", sections: [{ kind: "coverage-radar" }] }),
    );
    for (const g of GROUPS) {
      for (const k of g.card.preview) expect(keys.has(k), k).toBe(true);
    }
  });
});
