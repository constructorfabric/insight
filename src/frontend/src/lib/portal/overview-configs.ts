import type { DisplayText } from "@/i18n/localized-text";
import { headlineMetricKeys } from "@/lib/insight/groups";
import { sectionMetricKeys, type LensConfig } from "@/lib/portal/lens-configs";
import { defaultZoneItem } from "@/lib/portal/nav-model";

/**
 * The Overview zone registry: each pane item (nav-model ZONE_SECTIONS.overview)
 * maps to a LensConfig rendered by DomainLensView — same pattern as
 * DIRECTION_LENSES (design DESIGN-2026-07-27-overview §3/§5). Item ids must
 * match nav-model verbatim (pinned by overview-configs.test.ts).
 */

/** Attention scans the cross-domain headline set — every group's card preview. */
const ATTENTION_KEYS: readonly string[] = headlineMetricKeys();

/** The item the router renders when no pane item is selected. */
export const DEFAULT_OVERVIEW_ITEM = defaultZoneItem("overview")!;

export const OVERVIEW_ITEMS: Record<string, LensConfig> = {
  [DEFAULT_OVERVIEW_ITEM]: {
    title: { key: "overview.title" },
    tagline: { key: "overview.tagline_rollup" },
    sections: [
      {
        kind: "headline",
        // Task throughput belongs in a cross-functional headline: without it
        // the org's top line was three-quarters code and cost, and the work
        // that never touches a repo went unrepresented.
        metrics: [
          "git.default_branch_prs_merged",
          "git.pr_size",
          "tasks.closed",
          "collab.messages_sent",
          "ai.cost",
          // The daily distribution: this row totals a period, and only the
          // per-day steps add up to one. The month's running snapshot does not.
          "ai.daily_approximate_extra_usage_cost",
        ],
      },
      // The old header's "N using AI" stat, now an honest participation card
      // (count + share + delta). No trend section here → card renders chartless.
      {
        kind: "participation",
        metrics: ["ai.active_days"],
        title: "AI adoption",
        noun: "People using AI",
      },
      { kind: "attention", metrics: ATTENTION_KEYS, max: 8 },
      { kind: "direction-cards", variant: "compact" },
    ],
  },
  "by-direction": {
    title: { key: "overview.by_direction_title" },
    tagline: { key: "overview.tagline_next" },
    sections: [{ kind: "direction-cards", variant: "full" }],
  },
  attention: {
    title: { key: "worth_a_look.overview_title" },
    tagline: { key: "overview.tagline_who" },
    sections: [{ kind: "attention", metrics: ATTENTION_KEYS, max: 30 }],
  },
};

/** How an Overview item reads the viewer alone; an item absent here is team-only. */
export type OwnOverviewItem =
  | { kind: "home" }
  | { kind: "lens"; config: LensConfig }
  | { kind: "attention"; title: DisplayText; tagline: DisplayText };

export const OWN_OVERVIEW_ITEMS: Record<string, OwnOverviewItem> = {
  [DEFAULT_OVERVIEW_ITEM]: { kind: "home" },
  "by-direction": {
    kind: "lens",
    config: {
      title: { key: "overview.by_direction" },
      tagline: { key: "overview.tagline_own_direction" },
      sections: [{ kind: "direction-cards", variant: "full" }],
    },
  },
  attention: {
    kind: "attention",
    title: { key: "worth_a_look.title" },
    tagline: { key: "overview.tagline_own_attention" },
  },
};

/**
 * Union of metric keys across every Overview item — one stable grid query for
 * the whole zone, so switching pane items never re-spins (mirror of
 * directionMetricKeys).
 */
export function overviewMetricKeys(): string[] {
  const keys = new Set<string>();
  for (const config of Object.values(OVERVIEW_ITEMS)) {
    for (const k of sectionMetricKeys(config)) keys.add(k);
  }
  return [...keys];
}
