import { describe, expect, it } from "vitest";

import { GROUPS } from "@/lib/insight/groups";
import { DIRECTIONS } from "@/lib/portal/nav-model";
import { parseNavPaths } from "@/lib/portal/nav-policy";
import {
  DIRECTION_LENSES,
  directionMetricKeys,
  lensEntry,
  sectionMetricKeys,
  visibleSections,
  type LensConfig,
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

  it("puts each default-branch reading next to the total it refines", () => {
    const entry = lensEntry("dev", "Git output");
    const headline = (entry as LensConfig).sections.find(
      (section): section is Extract<SectionSpec, { kind: "headline" }> =>
        section.kind === "headline",
    );
    const metrics = headline?.metrics ?? [];

    // Adjacency is the rule, not the exact tile list: a split read apart from
    // its total says nothing, because "landed" only means something against
    // the whole. Asserting the whole array instead would fail on any unrelated
    // tile, under a name that would not explain why.
    for (const [total, split] of [
      ["git.commits", "git.default_branch_commits"],
      ["git.prs_merged", "git.default_branch_prs_merged"],
    ]) {
      const at = metrics.indexOf(total);
      expect(at, `${total} is in the headline`).toBeGreaterThanOrEqual(0);
      expect(metrics[at + 1], `the tile after ${total}`).toBe(split);
    }
  });

  it("explains a derived dimension where a label cannot", () => {
    const entry = lensEntry("dev", "Overview");
    const composition = (entry as LensConfig).sections.find(
      (section): section is Extract<SectionSpec, { kind: "composition" }> =>
        section.kind === "composition" && section.dimension === "category",
    );

    // The lead states the precedence, which is the part a reader cannot guess
    // from the labels, and every category the warehouse can emit is named.
    const notes = composition?.notes ?? [];
    expect(notes[0]).toMatch(/first rule that matches wins/i);
    for (const label of [
      "Vendored / Generated",
      "Tests",
      "Documentation",
      "Configuration",
      "Code",
    ]) {
      expect(notes.some((note) => note.startsWith(label)), label).toBe(true);
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
});

describe("visibleSections", () => {
  const gate = (planned: string[], hide: string[] = []) => ({
    hide: parseNavPaths(hide, "hide"),
    planned: parseNavPaths(planned, "planned"),
  });

  const config = {
    title: "Test",
    sections: [
      { kind: "headline", metrics: ["tasks.closed", "ai.cost"] },
      {
        kind: "stat-tiles",
        title: "Typical",
        metrics: ["tasks.resolution_time", "tasks.pickup_time"],
      },
      {
        kind: "participation",
        metrics: ["ai.active_days"],
        title: "AI adoption",
        noun: "People using AI",
      },
      {
        kind: "distribution",
        metric: "tasks.resolution_time",
        title: "Resolution",
        caption: "c",
        unitLabel: "u",
      },
      { kind: "direction-cards", variant: "compact" },
    ] as const satisfies readonly SectionSpec[],
  };

  it("returns the declared config untouched when the install gates no metric", () => {
    expect(visibleSections(config, false, gate(["zone:scorecard"]))).toBe(
      config
    );
  });

  it("drops a gated metric and keeps the rest of its section", () => {
    const gated = visibleSections(config, false, gate(["metric:ai.*"]));
    const headline = gated.sections[0];

    expect(headline).toMatchObject({
      kind: "headline",
      metrics: ["tasks.closed"],
    });
  });

  it("drops a section left with none of its own metrics", () => {
    const gated = visibleSections(config, false, gate(["metric:ai.*"]));

    expect(gated.sections.map((s) => s.kind)).not.toContain("participation");
  });

  it("drops a single-metric section whose metric is gated", () => {
    const gated = visibleSections(
      config,
      false,
      gate(["metric:tasks.resolution_time"])
    );

    expect(gated.sections.map((s) => s.kind)).not.toContain("distribution");
  });

  it("keeps the sections that name no metric of their own", () => {
    const gated = visibleSections(
      config,
      false,
      gate(["metric:ai.*", "metric:tasks.resolution_time"])
    );

    expect(gated.sections.map((s) => s.kind)).toContain("direction-cards");
  });

  it("shows every gated section to a reader who asked for planned ones", () => {
    const gated = visibleSections(config, true, gate(["metric:ai.*"]));

    expect(gated.sections).toEqual(config.sections);
  });
});
