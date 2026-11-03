import { describe, expect, it } from "vitest";

import { GROUPS, HEATMAP_METRIC_KEYS, KPI_ROW } from "@/lib/insight/groups";
import { OVERVIEW_ITEMS } from "@/lib/portal/overview-configs";
import { DIRECTIONS } from "@/lib/portal/nav-model";
import { parseNavPaths } from "@/lib/portal/nav-policy";
import {
  DIRECTION_LENSES,
  directionMetricKeys,
  lensEntry,
  sectionMetricKeys,
  tenantSectionMetricKeys,
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
        if ("comingSoon" in entry || "entity" in entry) continue;
        for (const key of sectionMetricKeys(entry)) {
          expect(KNOWN_KEYS.has(key), `${dir}/${lens}: ${key}`).toBe(true);
        }
      }
    }
  });

  it("keeps tenant lens keys out of every person surface", () => {
    for (const [dir, lenses] of Object.entries(DIRECTION_LENSES)) {
      const gridKeys = new Set(directionMetricKeys(dir));
      for (const [lens, entry] of Object.entries(lenses)) {
        if (!("entity" in entry)) continue;
        for (const key of tenantSectionMetricKeys(entry)) {
          // A tenant-entity key inside a person-entity request fails the whole
          // batch, and GROUPS drive person surfaces — both must stay clean.
          expect(gridKeys.has(key), `${dir}/${lens}: ${key} in grid`).toBe(false);
          expect(KNOWN_KEYS.has(key), `${dir}/${lens}: ${key} in GROUPS`).toBe(false);
          expect(key, `${dir}/${lens}: ${key} shape`).toMatch(/^[a-z_]+\.[a-z0-9_]+$/);
        }
      }
    }
  });

  it("reads git work on the default branch only, on every surface", () => {
    const anyBranch = new Set([
      "git.commits",
      "git.prs_created",
      "git.prs_merged",
      "git.lines_added",
      "git.lines_removed",
      "git.code_lines",
      "git.active_days",
      "git.commits_per_active_day",
      "git.merge_rate",
      "git.non_default_branch_commits",
      "git.non_default_branch_prs_created",
      "git.non_default_branch_prs_merged",
      "git.non_default_branch_lines_added",
      "git.non_default_branch_lines_removed",
      "git.non_default_branch_code_lines",
    ]);
    const surfaces: Array<[string, readonly string[]]> = [
      ["dev lenses", directionMetricKeys("dev")],
      ...Object.entries(OVERVIEW_ITEMS).map(
        ([item, config]) => [`overview ${item}`, sectionMetricKeys(config)] as [string, string[]],
      ),
      ...GROUPS.flatMap((group) => [
        [`${group.id} collection`, group.collection.metrics.map((m) => m.key)] as [string, string[]],
        [`${group.id} preview`, group.card.preview] as [string, readonly string[]],
        [
          `${group.id} drilldown`,
          group.drilldown.flatMap((block) => ("metrics" in block ? block.metrics : [])),
        ] as [string, string[]],
      ]),
      ["KPI row", KPI_ROW],
      ["heatmap", HEATMAP_METRIC_KEYS],
    ];

    for (const [surface, keys] of surfaces) {
      expect(keys.filter((key) => anyBranch.has(key)), surface).toEqual([]);
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
        const keys =
          "entity" in entry
            ? tenantSectionMetricKeys(entry)
            : sectionMetricKeys(entry);
        expect(keys.length).toBeLessThanOrEqual(50);
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
  it("collects dimension-table and ownership metrics", () => {
    const keys = sectionMetricKeys({
      title: "t",
      sections: [
        {
          kind: "dimension-table",
          title: "t",
          dimension: "repository",
          noun: "repositories",
          metrics: ["git.prs_merged", "git.lines_added"],
        },
        {
          kind: "ownership",
          metric: "git.code_lines",
          dimension: "repository",
          title: "t",
        },
      ],
    });
    expect(keys.sort()).toEqual([
      "git.code_lines",
      "git.lines_added",
      "git.prs_merged",
    ]);
  });
  it("derives direction-cards keys from every configured Overview lens headline", () => {
    const keys = sectionMetricKeys({
      title: "t",
      sections: [{ kind: "direction-cards", variant: "full" }],
    });
    expect(keys).toContain("git.default_branch_commits");
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

  it("gates dimension-table columns and drops an emptied table or ownership section", () => {
    const repoConfig = {
      title: "T",
      sections: [
        {
          kind: "dimension-table",
          title: "t",
          dimension: "repository",
          noun: "repositories",
          metrics: ["git.prs_merged", "ai.cost"],
        },
        {
          kind: "ownership",
          metric: "ai.cost",
          dimension: "repository",
          title: "o",
        },
      ] as const satisfies readonly SectionSpec[],
    };
    const gated = visibleSections(repoConfig, false, gate(["metric:ai.*"]));
    expect(gated.sections).toEqual([
      {
        kind: "dimension-table",
        title: "t",
        dimension: "repository",
        noun: "repositories",
        metrics: ["git.prs_merged"],
      },
    ]);
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
