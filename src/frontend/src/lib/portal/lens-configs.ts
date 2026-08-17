import { DIRECTIONS, type Direction, type Readiness } from "@/lib/portal/nav-model";

/**
 * The Directions registry: every direction × lens maps to either a LensConfig
 * (rendered by DomainLensView from typed sections) or an honest ComingSoon
 * note naming what would enable it (design D1, rule 9).
 *
 * Metric MEANING stays server-owned; configs carry keys + section composition
 * only. No named individuals anywhere (design D4/rule 10).
 */

export type ConcentrationFraming = "bus-factor" | "load-balance";

export type SectionSpec =
  | { kind: "headline"; metrics: readonly string[] }
  | { kind: "stat-tiles"; title: string; metrics: readonly string[] }
  | { kind: "trend"; metrics: readonly string[] }
  | { kind: "distribution"; metric: string; title: string; caption: string; unitLabel: string }
  | { kind: "concentration"; metrics: readonly string[]; framing: ConcentrationFraming }
  | { kind: "composition"; metric: string; dimension: string; title: string }
  // Flow-depth sections: event-histogram merges per-entity server bins when
  // edges align (they don't on the current API — honest fallback, see design §7);
  // participation counts active people.
  | { kind: "event-histogram"; metric: string; title: string }
  | { kind: "participation"; metrics: readonly string[]; title: string; noun: string }
  // Overview-motivated, zone-agnostic sections (design DESIGN-2026-07-27-overview §4).
  | { kind: "attention"; metrics: readonly string[]; max: number }
  | { kind: "direction-cards"; variant: "compact" | "full" }
  // How much of each person's work we can see at all, and for how many
  // people (#2408). Fetches its own period-only collection across every
  // group, so it contributes no keys to the zone grid.
  | { kind: "coverage-levels" };

export interface LensConfig {
  title: string;
  /** Subtitle tail after "N people · " (defaults to "trend & balance"). */
  tagline?: string;
  sections: readonly SectionSpec[];
  /** Whole-tab message when no metric of the lens is observed (rule 6). */
  notIngested?: string;
}

/**
 * A lens that renders nothing yet, and WHY — the two causes must not look
 * alike. `planned`: the product does not model this metric family yet (same
 * for every tenant). `unbuilt`: the data path exists and the screen is ours to
 * build. See `Readiness` in nav-model.
 */
export interface LensRoadmap {
  comingSoon: string;
  readiness: Readiness;
}

/** Either a lens we render or one we only name — the pane treats the two differently. */
export type LensEntry = LensConfig | LensRoadmap;

/** The registry entry for a direction's lens — a config, a roadmap note, or nothing. */
export function lensEntry(dir: string, lens: string): LensEntry | undefined {
  return DIRECTION_LENSES[dir]?.[lens];
}

/** A direction's lenses in pane order, minus the roadmap ones a reader opted out of. */
export function visibleLenses(
  direction: Direction,
  showPlanned: boolean,
): string[] {
  return direction.lenses.filter((lens) => {
    const entry = lensEntry(direction.id, lens);
    return !entry || !("comingSoon" in entry) || showPlanned;
  });
}

/**
 * Directions worth listing: a branch whose every lens is filtered out expands
 * into nothing, so it is a dead end rather than a place to look.
 */
export function visibleDirections(showPlanned: boolean): Direction[] {
  return DIRECTIONS.filter((d) => visibleLenses(d, showPlanned).length > 0);
}

/** Unique metric keys a config needs in its period+peer grid. */
export function sectionMetricKeys(config: LensConfig): string[] {
  const keys = new Set<string>();
  for (const s of config.sections) {
    switch (s.kind) {
      case "headline":
      case "stat-tiles":
      case "trend":
      case "concentration":
      case "participation":
      case "attention":
        for (const k of s.metrics) keys.add(k);
        break;
      case "distribution":
      case "composition":
      case "event-histogram":
        keys.add(s.metric);
        break;
      case "direction-cards":
        // Cards derive from every configured direction Overview lens (design O4).
        for (const lenses of Object.values(DIRECTION_LENSES)) {
          const overview = lenses["Overview"];
          if (!overview || "comingSoon" in overview) continue;
          for (const sec of overview.sections) {
            if (sec.kind === "headline") for (const k of sec.metrics) keys.add(k);
          }
        }
        break;
      case "coverage-levels":
        // Deliberately none. Coverage asks whether ANY metric of a group
        // reads, so it needs every group's keys rather than the zone's
        // chosen few — widening the shared grid to that would make one tab
        // pay for all of them. It fetches its own period-only collection.
        break;
      default: {
        const _exhaustive: never = s;
        throw new Error(`Unhandled section kind: ${JSON.stringify(_exhaustive)}`);
      }
    }
  }
  return [...keys];
}

/* ── Development ─────────────────────────────────────────────────────── */

/** Product-side gap: the metric family is not in the semantic layer yet. */
const DEV_PLANNED = (what: string): LensRoadmap => ({
  comingSoon: `${what} — not available yet.`,
  readiness: "planned",
});

/**
 * Our gap: the dimensions this needs already ride on the git observations
 * (repository / project / file_extension / change_type), so this is frontend
 * work we owe, not a data request. Worded so nobody schedules a metric task.
 */
const DEV_UNBUILT = (what: string): LensRoadmap => ({
  comingSoon: `${what} — the data is there (git observations carry the dimensions); this view is still in development.`,
  readiness: "unbuilt",
});

const DEV: Record<string, LensEntry> = {
  Overview: {
    title: "Development",
    tagline: "output, flow & balance",
    sections: [
      { kind: "headline", metrics: ["git.commits", "git.prs_merged", "git.lines_added"] },
      {
        kind: "stat-tiles",
        title: "Typical values (median)",
        metrics: ["git.pr_cycle_time_h", "git.pr_size", "git.merge_rate"],
      },
      { kind: "trend", metrics: ["git.commits", "git.prs_merged"] },
      { kind: "concentration", metrics: ["git.commits"], framing: "bus-factor" },
      {
        kind: "composition",
        metric: "git.lines_added",
        dimension: "category",
        title: "Lines by category",
      },
    ],
  },
  "Git output": {
    title: "Development · Git output",
    sections: [
      {
        kind: "headline",
        metrics: ["git.commits", "git.prs_created", "git.prs_merged", "git.code_lines"],
      },
      { kind: "trend", metrics: ["git.commits", "git.prs_merged"] },
      {
        kind: "distribution",
        metric: "git.commits",
        title: "How many commits people made",
        caption:
          "How many people fall in each commit-count band — when the bars stretch far to the right, a few people account for most of it.",
        unitLabel: "commits per person",
      },
      { kind: "concentration", metrics: ["git.commits"], framing: "bus-factor" },
      {
        kind: "composition",
        metric: "git.lines_added",
        dimension: "repository",
        title: "Lines by repository",
      },
    ],
  },
  Flow: {
    title: "Development · Flow",
    tagline: "how smoothly work moves",
    sections: [
      {
        kind: "stat-tiles",
        title: "Typical values (median)",
        metrics: [
          "git.pr_cycle_time_h",
          "git.pr_size",
          "git.commit_size",
          "git.merge_rate",
          "git.commits_per_active_day",
        ],
      },
      {
        kind: "event-histogram",
        metric: "git.pr_cycle_time_h",
        title: "How long pull requests stayed open",
      },
    ],
  },
  Delivery: {
    title: "Development · Delivery",
    notIngested: "Jira is not connected yet.",
    sections: [
      { kind: "headline", metrics: ["tasks.closed", "tasks.bugs_fixed"] },
      {
        kind: "stat-tiles",
        title: "Typical task times (median)",
        metrics: ["tasks.resolution_time", "tasks.pickup_time", "tasks.dev_time"],
      },
      { kind: "trend", metrics: ["tasks.closed", "tasks.bugs_fixed"] },
      {
        kind: "distribution",
        metric: "tasks.closed",
        title: "How many tasks people closed",
        caption: "Each bar is a range of tasks closed, and how many people fall in it.",
        unitLabel: "tasks closed per person",
      },
    ],
  },
  Activity: DEV_PLANNED("Per-person activity-day metrics"),
  Quality: DEV_PLANNED("Review / reopen quality metrics"),
  Continuity: DEV_PLANNED("Longitudinal continuity metrics"),
  Repositories: DEV_UNBUILT("Repository-level rollups"),
  Elements: DEV_UNBUILT("Element-level (file/module) analytics"),
};

/* ── Collaboration (ported unchanged from ModalityView configs) ──────── */

const COLLAB: Record<string, LensEntry> = {
  Overview: {
    title: "Collaboration",
    sections: [
      {
        kind: "headline",
        metrics: ["collab.messages_sent", "collab.meeting_hours", "collab.focus_time_pct"],
      },
      { kind: "trend", metrics: ["collab.messages_sent", "collab.meeting_hours"] },
      {
        kind: "distribution",
        metric: "collab.meeting_hours",
        title: "How many hours people spent in meetings",
        caption:
          "How many people fall in each meeting-hours band — a long right tail means a few people carry an outsized meeting load.",
        unitLabel: "meeting hours per person",
      },
      {
        kind: "concentration",
        metrics: ["collab.meeting_hours", "collab.messages_sent"],
        framing: "load-balance",
      },
    ],
  },
  Messaging: {
    title: "Messaging",
    sections: [
      {
        kind: "headline",
        metrics: ["collab.messages_sent", "collab.msgs_per_active_day", "collab.active_days"],
      },
      { kind: "trend", metrics: ["collab.messages_sent"] },
      {
        kind: "distribution",
        metric: "collab.messages_sent",
        title: "How many messages people sent",
        caption:
          "How many people fall in each message-volume band — a long right tail means a few people account for most of the chatter.",
        unitLabel: "messages per person",
      },
      { kind: "concentration", metrics: ["collab.messages_sent"], framing: "load-balance" },
    ],
  },
  Meetings: {
    title: "Meetings",
    sections: [
      {
        kind: "headline",
        metrics: ["collab.meeting_hours", "collab.meetings_count", "collab.meeting_free_days"],
      },
      { kind: "trend", metrics: ["collab.meeting_hours"] },
      {
        kind: "distribution",
        metric: "collab.meeting_hours",
        title: "How many hours people spent in meetings",
        caption:
          "How many people fall in each meeting-hours band — a long right tail means a few people carry an outsized meeting load.",
        unitLabel: "meeting hours per person",
      },
      { kind: "concentration", metrics: ["collab.meeting_hours"], framing: "load-balance" },
    ],
  },
  // emails_received deliberately omitted: distribution-list/CI noise (see git history).
  Email: {
    title: "Email",
    sections: [
      { kind: "headline", metrics: ["collab.emails_sent", "collab.emails_read"] },
      { kind: "trend", metrics: ["collab.emails_sent"] },
      {
        kind: "distribution",
        metric: "collab.emails_sent",
        title: "How many emails people sent",
        caption:
          "How many people fall in each sent-email band — a long right tail means a few people send most of the email.",
        unitLabel: "emails sent per person",
      },
      { kind: "concentration", metrics: ["collab.emails_sent"], framing: "load-balance" },
    ],
  },
  "Focus time": {
    title: "Focus time",
    sections: [
      { kind: "headline", metrics: ["collab.focus_time_pct", "collab.meeting_free_days"] },
      {
        kind: "distribution",
        metric: "collab.focus_time_pct",
        title: "How much focus time people had",
        caption:
          "How many people fall in each focus-time band — a cluster on the left means many people have little uninterrupted focus time.",
        unitLabel: "focus time (share of working time) per person",
      },
    ],
  },
  "Files & sharing": {
    title: "Files & sharing",
    sections: [
      {
        kind: "headline",
        metrics: ["collab.files_shared", "collab.files_engaged", "collab.files_shared_external"],
      },
      { kind: "trend", metrics: ["collab.files_shared"] },
      {
        kind: "distribution",
        metric: "collab.files_shared",
        title: "How many files people shared",
        caption:
          "How many people fall in each files-shared band — a long right tail means a few people do most of the sharing.",
        unitLabel: "files shared per person",
      },
      { kind: "concentration", metrics: ["collab.files_shared"], framing: "load-balance" },
    ],
  },
};

/* ── Knowledge / Wiki ────────────────────────────────────────────────── */

const WIKI: Record<string, LensEntry> = {
  Overview: {
    title: "Knowledge / Wiki",
    sections: [
      { kind: "headline", metrics: ["wiki.pages_created", "wiki.edits", "wiki.comments"] },
      { kind: "trend", metrics: ["wiki.pages_created", "wiki.edits"] },
      {
        kind: "distribution",
        metric: "wiki.edits",
        title: "How many wiki edits people made",
        caption:
          "How many people fall in each wiki-edits band — a long right tail means knowledge writing is concentrated in a few hands.",
        unitLabel: "wiki edits per person",
      },
      { kind: "concentration", metrics: ["wiki.edits"], framing: "bus-factor" },
    ],
  },
  Authoring: {
    title: "Wiki · Authoring",
    sections: [
      { kind: "headline", metrics: ["wiki.pages_created", "wiki.pages_edited"] },
      {
        kind: "distribution",
        metric: "wiki.pages_created",
        title: "How many pages people created",
        caption: "Each bar is a range of pages created, and how many people fall in it.",
        unitLabel: "pages created per person",
      },
      { kind: "concentration", metrics: ["wiki.pages_created"], framing: "bus-factor" },
    ],
  },
  "Edits & comments": {
    title: "Wiki · Edits & comments",
    sections: [
      { kind: "headline", metrics: ["wiki.edits", "wiki.comments"] },
      { kind: "trend", metrics: ["wiki.edits", "wiki.comments"] },
      {
        kind: "distribution",
        metric: "wiki.edits",
        title: "How many wiki edits people made",
        caption: "Each bar is a range of wiki edits, and how many people fall in it.",
        unitLabel: "wiki edits per person",
      },
    ],
  },
  "Active authors": {
    title: "Wiki · Active authors",
    tagline: "who writes at all",
    sections: [
      {
        kind: "participation",
        metrics: ["wiki.pages_created", "wiki.edits", "wiki.comments"],
        title: "Participation",
        noun: "Active authors",
      },
      // Participation's per-bucket count reads the trend query; wiki.edits
      // dominates wiki activity, so the trend fetch keys off it alone — do NOT
      // add all three (row budget at org scope). The headline "N of M" uses the
      // period grid over all three metrics.
      { kind: "trend", metrics: ["wiki.edits"] },
    ],
  },
};

/* ── Sales / Support (bullet-only directions) ────────────────────────── */

const SALES_NOTE: LensRoadmap = {
  comingSoon: "HubSpot data is not available yet.",
  readiness: "planned",
};
const SUPPORT_NOTE: LensRoadmap = {
  comingSoon: "Zendesk data is not available yet.",
  readiness: "planned",
};

const SALES: Record<string, LensEntry> = Object.fromEntries(
  ["Pipeline", "Deal flow", "Activity", "Velocity & quality"].map((l) => [
    l,
    SALES_NOTE,
  ]),
);
const SUPPORT: Record<string, LensEntry> = Object.fromEntries(
  ["Tickets", "CSAT", "Knowledge base", "Comments & updates"].map((l) => [
    l,
    SUPPORT_NOTE,
  ]),
);

export const DIRECTION_LENSES: Record<string, Record<string, LensEntry>> = {
  dev: DEV,
  collab: COLLAB,
  wiki: WIKI,
  sales: SALES,
  support: SUPPORT,
};

/** Union of metric keys across every configured lens of a direction — one
 * stable grid collection per direction so switching lenses never changes the
 * query key (no spinner). ComingSoon entries contribute nothing. */
export function directionMetricKeys(dir: string): string[] {
  const keys = new Set<string>();
  for (const entry of Object.values(DIRECTION_LENSES[dir] ?? {})) {
    if ("comingSoon" in entry) continue;
    for (const k of sectionMetricKeys(entry)) keys.add(k);
  }
  return [...keys];
}
