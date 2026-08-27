import {
  directionHidden,
  directionPlanned,
  gatesAnyMetric,
  lensHidden,
  lensPlanned,
  metricVisible,
  navPolicy,
  visibleMetricKeys,
  type InstanceNavPolicy,
} from "@/lib/portal/nav-policy";
import {
  DIRECTIONS,
  lensSlug,
  type Direction,
} from "@/lib/portal/nav-model";

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
  | {
      kind: "trend";
      metrics: readonly string[];
      /**
       * Chart the roster's active contributors alongside the totals, derived
       * from this metric's own per-person rows. A total says how much; this
       * says how many people it came from.
       */
      activeContributorsFor?: string;
    }
  | {
      kind: "distribution";
      metric: string;
      title: string;
      caption: string;
      unitLabel: string;
    }
  | {
      kind: "concentration";
      metrics: readonly string[];
      framing: ConcentrationFraming;
    }
  // `splitBy` cuts each bar into segments of a second dimension — the same
  // total, showing what it is made of.
  | {
      kind: "composition";
      metric: string;
      dimension: string;
      title: string;
      splitBy?: string;
      /**
       * Lines shown under the bars, for a dimension whose values are derived
       * rather than reported — a reader cannot tell how a bucket was decided
       * from its label alone. One line each, the first is the lead.
       */
      notes?: readonly string[];
    }
  // Flow-depth sections: event-histogram merges per-entity server bins when
  // edges align (they don't on the current API — honest fallback, see design §7);
  // participation counts active people.
  | { kind: "event-histogram"; metric: string; title: string }
  // One row per value of a dimension, columns from the listed metrics — the
  // rollup view collapses the person grain server-side. The FIRST metric ranks
  // the rows and supplies the distinct-person count column.
  | {
      kind: "dimension-table";
      title: string;
      dimension: string;
      metrics: readonly string[];
      /** What one row is, for the section label's count ("repositories"). */
      noun: string;
      /** Rows shown before the rest folds into a remainder line. */
      limit?: number;
    }
  // Concentration risk per dimension value: how much of the metric the top
  // contributors carry inside each value (bus factor per repository). Shares
  // are computed from the per-person breakdown; nobody is named.
  | {
      kind: "ownership";
      metric: string;
      dimension: string;
      title: string;
    }
  | {
      kind: "participation";
      metrics: readonly string[];
      title: string;
      noun: string;
    }
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

export interface LensRoadmap {
  comingSoon: string;
}

/**
 * A lens over ORG-grain (tenant-entity) metrics — CI is the first. Rendered by
 * TenantLensView: no roster, no peer view, no per-person anything; the entity
 * is the organization and every value is one number per period/bucket/row.
 */
export interface TenantLensConfig {
  title: string;
  tagline?: string;
  entity: "tenant";
  sections: readonly TenantSectionSpec[];
  /** Whole-tab message when no metric of the lens is observed (rule 6). */
  notIngested: string;
}

export type TenantSectionSpec =
  | { kind: "headline"; metrics: readonly string[] }
  | { kind: "stat-tiles"; title: string; metrics: readonly string[] }
  // One chart per section: volume and rate never share an axis, so a lens
  // draws them as two sections rather than one chart with two scales.
  | {
      kind: "trend";
      title: string;
      description?: string;
      metrics: readonly string[];
      plot?: "line" | "step" | "area";
      /** Draw the window average of the FIRST metric as a reference rule. */
      referenceMean?: boolean;
      /** List buckets that fell 2σ below their trailing mean under the chart. */
      flagOutliers?: boolean;
    }
  | {
      kind: "composition";
      metric: string;
      dimension: string;
      title: string;
      splitBy?: string;
      /** Reader-facing names for split values the vendor words misleadingly. */
      segmentLabels?: Readonly<Record<string, string>>;
    }
  | { kind: "histogram"; metric: string; title: string; caption: string }
  // Weekly columns of one metric cut by a dimension; share=true draws the
  // same data as a 100% composition instead of absolute volume.
  | {
      kind: "stacked-trend";
      metric: string;
      splitBy: string;
      title: string;
      description?: string;
      share?: boolean;
      segmentLabels?: Readonly<Record<string, string>>;
    }
  // A grid of small line charts, one per dimension value, identical y-axis.
  | {
      kind: "small-multiples";
      metric: string;
      dimension: string;
      title: string;
      top?: number;
    }
  // Two measures per dimension value at once; size (optional) is a third.
  // quadrants=true rules the medians and labels the four cells.
  | {
      kind: "scatter";
      x: string;
      y: string;
      size?: string;
      dimension: string;
      title: string;
      description?: string;
      quadrants?: boolean;
    }
  // Day-of-week × two-hour block magnitude ramp of one metric.
  | { kind: "heatmap-hours"; metric: string; title: string }
  // Per-hour-block columns of a rate with a ±1σ band around its mean.
  | { kind: "hour-columns"; metric: string; title: string; description?: string }
  // First half of the window against the second, one slope per dimension value.
  | {
      kind: "slope";
      metric: string;
      dimension: string;
      title: string;
      description?: string;
    }
  // Signed change between the window's two halves, one bar per value.
  | {
      kind: "momentum";
      metric: string;
      dimension: string;
      title: string;
      description?: string;
    }
  // Derived signals over the gate population (computed from ci.runs rows):
  // what fixing the worst pipelines would buy.
  | { kind: "marginal-impact"; title: string; description?: string }
  // The org headline rate against the unweighted mean over a dimension.
  | { kind: "callout-pair"; metric: string; dimension: string; title: string }
  // Median of one metric split two ways per dimension value (fail vs pass).
  | {
      kind: "dumbbell";
      metric: string;
      dimension: string;
      splitBy: string;
      left: string;
      right: string;
      title: string;
      description?: string;
    }
  // Cumulative share curve of one metric over a ranked dimension.
  | { kind: "cumulative"; metric: string; dimension: string; title: string; description?: string }
  // One 100% bar of a metric split by a dimension.
  | {
      kind: "decomposition";
      metric: string;
      splitBy: string;
      title: string;
      segmentLabels?: Readonly<Record<string, string>>;
    }
  // Mean weekly value and its volatility per dimension value, resolved to a
  // verdict (computed client-side from the weekly series).
  | {
      kind: "verdict-table";
      metric: string;
      dimension: string;
      title: string;
      description?: string;
      minWeeks?: number;
    };

/** Metric keys a tenant lens fetches — deliberately NOT part of the person
 *  grid union: a tenant-entity key inside a person-entity request would fail
 *  the whole batch. */
export function tenantSectionMetricKeys(config: TenantLensConfig): string[] {
  const keys = new Set<string>();
  for (const s of config.sections) {
    switch (s.kind) {
      case "headline":
      case "stat-tiles":
      case "trend":
        for (const k of s.metrics) keys.add(k);
        break;
      case "composition":
      case "histogram":
      case "stacked-trend":
      case "small-multiples":
      case "heatmap-hours":
      case "hour-columns":
      case "slope":
      case "momentum":
      case "callout-pair":
      case "dumbbell":
      case "cumulative":
      case "decomposition":
      case "verdict-table":
        keys.add(s.metric);
        break;
      case "scatter":
        keys.add(s.x);
        keys.add(s.y);
        if (s.size) keys.add(s.size);
        break;
      case "marginal-impact":
        // Derived from the ci.runs rows the lens fetches anyway.
        keys.add("ci.runs");
        break;
      default: {
        const _exhaustive: never = s;
        throw new Error(`Unhandled tenant section: ${JSON.stringify(_exhaustive)}`);
      }
    }
  }
  return [...keys];
}

/** Either a lens we render or one we only name — the pane treats the two differently. */
export type LensEntry = LensConfig | LensRoadmap | TenantLensConfig;

/** The registry entry for a direction's lens — a config, a roadmap note, or nothing. */
export function lensEntry(dir: string, lens: string): LensEntry | undefined {
  return DIRECTION_LENSES[dir]?.[lens];
}

export function lensRoadmap(
  direction: Direction,
  lens: string,
  policy: InstanceNavPolicy = navPolicy()
): boolean {
  return (
    directionPlanned(direction.id, policy) ||
    lensPlanned(direction.id, lensSlug(lens), policy)
  );
}

export function visibleLenses(
  direction: Direction,
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): string[] {
  return direction.lenses.filter((lens) => {
    if (lensHidden(direction.id, lensSlug(lens), policy)) return false;
    return !lensRoadmap(direction, lens, policy) || showPlanned;
  });
}

/**
 * Directions worth listing: a branch whose every lens is filtered out expands
 * into nothing, so it is a dead end rather than a place to look.
 */
export function visibleDirections(
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): Direction[] {
  return DIRECTIONS.filter(
    (d) =>
      !directionHidden(d.id, policy) &&
      visibleLenses(d, showPlanned, policy).length > 0
  );
}

export function overviewCardDirections(
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): Direction[] {
  return visibleDirections(showPlanned, policy).filter((d) =>
    visibleLenses(d, showPlanned, policy).includes("Overview")
  );
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
      case "ownership":
        keys.add(s.metric);
        break;
      case "dimension-table":
        for (const k of s.metrics) keys.add(k);
        break;
      case "direction-cards":
        // Cards derive from every configured direction Overview lens (design O4).
        for (const lenses of Object.values(DIRECTION_LENSES)) {
          const overview = lenses["Overview"];
          if (!overview || "comingSoon" in overview) continue;
          for (const sec of overview.sections) {
            if (sec.kind === "headline")
              for (const k of sec.metrics) keys.add(k);
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
        throw new Error(
          `Unhandled section kind: ${JSON.stringify(_exhaustive)}`
        );
      }
    }
  }
  return [...keys];
}

/**
 * The lens as this install shows it: sections lose the metrics it gates, and a
 * section left with none of its own is dropped rather than drawn empty.
 *
 * The data layer already refuses to fetch a gated metric, so a tile or a row
 * would simply not appear — but a section that draws its own frame (a title, a
 * participation card counting people rather than reading a value) would keep
 * standing over the hole. Composition is where that is settled.
 */
export function visibleSections(
  config: LensConfig,
  showPlanned: boolean,
  policy: InstanceNavPolicy = navPolicy()
): LensConfig {
  if (!gatesAnyMetric(policy)) return config;

  const sections: SectionSpec[] = [];
  for (const s of config.sections) {
    switch (s.kind) {
      case "headline":
      case "stat-tiles":
      case "trend":
      case "concentration":
      case "participation":
      case "attention": {
        const metrics = visibleMetricKeys(s.metrics, showPlanned, policy);
        if (metrics.length) sections.push({ ...s, metrics });
        break;
      }
      case "distribution":
      case "composition":
      case "event-histogram":
      case "ownership":
        if (metricVisible(s.metric, showPlanned, policy)) sections.push(s);
        break;
      case "dimension-table": {
        const metrics = visibleMetricKeys(s.metrics, showPlanned, policy);
        if (metrics.length) sections.push({ ...s, metrics });
        break;
      }
      case "direction-cards":
      case "coverage-levels":
        // Neither names a metric of its own: the cards read each direction's
        // own Overview lens (gated where that lens is composed) and coverage
        // counts sections.
        sections.push(s);
        break;
      default: {
        const _exhaustive: never = s;
        throw new Error(
          `Unhandled section kind: ${JSON.stringify(_exhaustive)}`
        );
      }
    }
  }
  return { ...config, sections };
}

/* ── Development ─────────────────────────────────────────────────────── */

/** Product-side gap: the metric family is not in the semantic layer yet. */
const PRODUCT_GAP = (what: string): LensRoadmap => ({
  comingSoon: `${what} — not available yet.`,
});

/**
 * Our gap: the dimensions this needs already ride on the git observations
 * (repository / project / file_extension / change_type), so this is frontend
 * work we owe, not a data request. Worded so nobody schedules a metric task.
 */
const SCREEN_GAP = (what: string): LensRoadmap => ({
  comingSoon: `${what} — the data is there (git observations carry the dimensions); this view is still in development.`,
});

/**
 * How the git file-category taxonomy is decided, in the order the warehouse
 * applies it (`git_file_category` in the dbt macros). Wording stays behavioural
 * — what lands in a bucket — rather than quoting the regexes, which change.
 *
 * Frontend copy on purpose: a dimension value carries a label over the wire and
 * nothing else, so there is no server field to put this in.
 */
const GIT_CATEGORY_NOTES = [
  "Every file gets one category, from its path — the first rule that matches wins.",
  "Vendored / Generated — dependency folders, build output, minified and generated files, lockfiles.",
  "Tests — files named *.test.* or *.spec.*, and anything under test/, tests/ or __tests__/.",
  "Documentation — .md, .rst and .adoc files, and anything under docs/.",
  "Configuration — .yaml, .toml, .cfg and .ini files, and lockfiles not already counted as generated.",
  "Code — whatever the other categories did not claim.",
] as const;

const DEV: Record<string, LensEntry> = {
  Overview: {
    title: "Development",
    tagline: "output, flow & balance",
    sections: [
      {
        kind: "headline",
        metrics: ["git.commits", "git.prs_merged", "git.lines_added"],
      },
      {
        kind: "stat-tiles",
        title: "Typical values (median)",
        metrics: ["git.pr_cycle_time_h", "git.pr_size", "git.merge_rate"],
      },
      { kind: "trend", metrics: ["git.commits", "git.prs_merged"] },
      {
        kind: "concentration",
        metrics: ["git.commits"],
        framing: "bus-factor",
      },
      {
        kind: "composition",
        metric: "git.lines_added",
        dimension: "category",
        title: "Lines by category",
        // The taxonomy is a path match in the warehouse, so the labels alone
        // do not say what landed where — and the order is the part a reader
        // cannot guess: one file has one category, decided by the first rule
        // that matches it.
        notes: GIT_CATEGORY_NOTES,
      },
    ],
  },
  "Git output": {
    title: "Development · Git output",
    sections: [
      {
        // Each split key sits beside the total it refines, so a reader meets
        // the two together rather than as unrelated tiles. Only the
        // default-branch side is drawn: the other half is the total minus it,
        // and naming both would spend four tiles on two facts.
        //
        // The two big figures are per ACTIVE person, each over its own active
        // count, so they are not a share of one another — the team totals
        // under them are what divide.
        kind: "headline",
        metrics: [
          "git.commits",
          "git.default_branch_commits",
          "git.prs_created",
          "git.prs_merged",
          "git.default_branch_prs_merged",
          "git.code_lines",
        ],
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
      {
        kind: "concentration",
        metrics: ["git.commits"],
        framing: "bus-factor",
      },
      {
        kind: "composition",
        metric: "git.lines_added",
        dimension: "repository",
        splitBy: "category",
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
      {
        kind: "event-histogram",
        metric: "git.pr_size",
        title: "How large pull requests were",
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
        metrics: [
          "tasks.resolution_time",
          "tasks.pickup_time",
          "tasks.dev_time",
        ],
      },
      { kind: "trend", metrics: ["tasks.closed", "tasks.bugs_fixed"] },
      {
        kind: "distribution",
        metric: "tasks.closed",
        title: "How many tasks people closed",
        caption:
          "Each bar is a range of tasks closed, and how many people fall in it.",
        unitLabel: "tasks closed per person",
      },
    ],
  },
  Activity: PRODUCT_GAP("Per-person activity-day metrics"),
  Quality: {
    title: "Development · Quality",
    tagline: "review hygiene & timing",
    sections: [
      {
        kind: "headline",
        metrics: ["git.reviews_performed", "git.pr_comments"],
      },
      {
        kind: "composition",
        metric: "git.pr_comments",
        dimension: "comment_target",
        title: "PR comments — own vs others' pull requests",
      },
      {
        kind: "stat-tiles",
        title: "Review hygiene (median across people)",
        metrics: [
          "git.review_coverage",
          "git.reviewers_per_pr",
          "git.multi_reviewer_rate",
          "git.merges_without_approval_rate",
        ],
      },
      {
        kind: "stat-tiles",
        title: "Review timing (median across people)",
        metrics: [
          "git.first_review_time_h",
          "git.first_review_time_p75_h",
          "git.review_to_merge_time_h",
          "git.approval_to_merge_time_h",
          "git.review_wait_share",
        ],
      },
      { kind: "trend", metrics: ["git.review_coverage"] },
      {
        kind: "event-histogram",
        metric: "git.first_review_time_h",
        title: "How long pull requests waited for the first review",
      },
      {
        kind: "stat-tiles",
        title: "Outcomes (median across people)",
        metrics: ["git.merge_rate", "git.pr_abandonment_rate"],
      },
    ],
  },
  // Org-grain (issue #2803): a pipeline run belongs to the organization, so
  // this lens renders tenant-entity metrics — no roster, no peer, no person.
  CI: {
    title: "Development · CI",
    tagline: "pipelines, org-wide",
    entity: "tenant",
    notIngested:
      "No CI metrics here yet — tenant metrics are opt-in per installation (metricCatalog.tenantMetricsEnabled), and a GitHub source must be collecting runs.",
    sections: [
      {
        kind: "headline",
        metrics: ["ci.gate_pass_rate", "ci.runs", "ci.gate_first_try_pass_rate"],
      },
      {
        kind: "stat-tiles",
        title: "Gate run durations & retries",
        metrics: [
          "ci.run_duration_min",
          "ci.run_duration_min_p90",
          "ci.run_duration_min_stddev",
          "ci.gate_retry_share",
        ],
      },
      {
        kind: "trend",
        title: "Runs over time",
        description: "Decided runs, all triggers.",
        metrics: ["ci.runs"],
        plot: "area",
      },
      {
        kind: "trend",
        title: "Gate pass rate over time",
        description:
          "Success share of commit-triggered runs; first-try shows what green cost in retries.",
        metrics: ["ci.gate_pass_rate", "ci.gate_first_try_pass_rate"],
      },
      {
        kind: "composition",
        metric: "ci.runs",
        dimension: "repository",
        splitBy: "outcome",
        title: "Runs by repository",
      },
      {
        kind: "composition",
        metric: "ci.run_hours",
        dimension: "repository",
        splitBy: "outcome",
        title: "CI hours by repository",
      },
      // Data-trust panel: how far CI joins to git history. PR runs build
      // synthetic merge refs the commit stream never sees, so the matched
      // line sitting far below runs is the known join defect (#2803) made
      // visible — the reader learns how much any commit-joined CI reading
      // can be trusted, per bucket.
      {
        kind: "trend",
        title: "CI ↔ commit join coverage",
        description:
          "Runs, collected commits, and runs whose head commit was collected — the gap is data the two streams cannot join.",
        metrics: [
          "ci.runs",
          "ci.commits_observed",
          "ci.runs_matched_commit",
        ],
      },
      {
        kind: "histogram",
        metric: "ci.run_duration_min",
        title: "How long a gate run takes",
        caption:
          "Each bar is a duration range, and how many runs fall in it. The tail is what people wait for.",
      },
      {
        kind: "composition",
        metric: "ci.deployments",
        dimension: "environment",
        splitBy: "outcome",
        title: "Deployments by environment",
        // GitHub marks a deployment superseded by a newer one "inactive" —
        // reading that as broken would make every busy environment look red.
        segmentLabels: { inactive: "superseded" },
      },
      {
        kind: "stacked-trend",
        metric: "ci.runs",
        splitBy: "outcome",
        title: "Run outcomes over time",
        description: "Every decided run classified, nothing dropped.",
      },
      {
        kind: "stacked-trend",
        metric: "ci.runs",
        splitBy: "outcome",
        share: true,
        title: "Outcome mix as shares",
        description: "The same runs, composition instead of volume.",
      },
      {
        kind: "trend",
        title: "Median gate run duration",
        description: "A step reads as \"held at\", not \"drifted to\".",
        metrics: ["ci.run_duration_min"],
        plot: "step",
      },
      {
        kind: "trend",
        title: "Gate pass rate against its window mean",
        description:
          "The rule is the window average; flagged buckets fell 2σ below their own trailing mean.",
        metrics: ["ci.gate_pass_rate"],
        referenceMean: true,
        flagOutliers: true,
      },
      {
        kind: "small-multiples",
        metric: "ci.runs",
        dimension: "repository",
        title: "Runs per repository",
        top: 12,
      },
      {
        kind: "slope",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "Pass rate, first half against second half",
        description: "The window split at its midpoint; direction labelled, not colour-only.",
      },
      {
        kind: "momentum",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "Momentum: second half against the first",
        description: "Change in pass rate, in points.",
      },
      {
        kind: "scatter",
        x: "ci.runs",
        y: "ci.gate_pass_rate",
        size: "ci.run_hours",
        dimension: "repository",
        title: "Volume against reliability",
        description: "Bubble area is CI hours consumed.",
      },
      {
        kind: "scatter",
        x: "ci.run_duration_min",
        y: "ci.gate_pass_rate",
        size: "ci.run_hours",
        dimension: "pipeline",
        title: "Slow and red — where to spend the effort",
        description: "Median duration against pass rate; bubble area is CI hours.",
        quadrants: true,
      },
      {
        kind: "heatmap-hours",
        metric: "ci.runs",
        title: "When CI actually runs",
      },
      {
        kind: "hour-columns",
        metric: "ci.gate_pass_rate",
        title: "Is CI riskier at some hours?",
        description: "Pass rate by two-hour block of the run's start, UTC; the band is ±1σ around the mean.",
      },
      {
        kind: "marginal-impact",
        title: "What fixing the worst pipelines would buy",
        description:
          "Gate pass rate if the top N failing pipelines produced no failures.",
      },
      {
        kind: "callout-pair",
        metric: "ci.gate_pass_rate",
        dimension: "repository",
        title: "Headline rate versus typical repository",
      },
      {
        kind: "dumbbell",
        metric: "ci.run_duration_min",
        dimension: "pipeline",
        splitBy: "outcome",
        left: "failure",
        right: "success",
        title: "Does a pipeline fail fast, or fail slow?",
        description: "Median duration of failing runs against passing runs.",
      },
      {
        kind: "cumulative",
        metric: "ci.run_hours",
        dimension: "pipeline",
        title: "Compute concentrates harder than failure does",
        description: "Cumulative share of all CI hours across pipelines, busiest first.",
      },
      {
        kind: "decomposition",
        metric: "ci.run_hours",
        splitBy: "outcome",
        title: "Where the CI hours actually went",
      },
      {
        kind: "verdict-table",
        metric: "ci.gate_pass_rate",
        dimension: "pipeline",
        title: "Stability verdict per pipeline",
        description:
          "Mean weekly pass rate and its volatility resolve to a verdict; thin histories stay unjudged.",
        minWeeks: 5,
      },
    ],
  },
  Continuity: PRODUCT_GAP("Longitudinal continuity metrics"),
  Repositories: {
    title: "Development · Repositories",
    tagline: "activity, reach & risk",
    sections: [
      {
        kind: "headline",
        metrics: [
          "git.prs_merged",
          "git.default_branch_prs_merged",
          "git.default_branch_code_lines",
          "git.non_default_branch_code_lines",
        ],
      },
      {
        kind: "trend",
        metrics: ["git.prs_created", "git.prs_merged"],
        activeContributorsFor: "git.prs_merged",
      },
      {
        kind: "stat-tiles",
        title: "Typical values (median)",
        metrics: [
          "git.pr_cycle_time_h",
          "git.pr_cycle_time_p75_h",
          "git.pr_size",
          "git.pr_commits",
          "git.merge_rate",
          "git.commits_per_active_day",
        ],
      },
      {
        kind: "composition",
        metric: "git.code_lines",
        dimension: "repository",
        splitBy: "branch_scope",
        title: "Lines by repository — default vs other branches",
      },
      {
        kind: "composition",
        metric: "git.prs_merged",
        dimension: "repository",
        title: "PRs merged by repository",
      },
      {
        kind: "dimension-table",
        title: "Repositories · ranked by PRs merged",
        dimension: "repository",
        noun: "repositories",
        metrics: [
          "git.prs_merged",
          "git.default_branch_prs_merged",
          "git.lines_added",
          "git.lines_removed",
          "git.pr_cycle_time_h",
          "git.pr_commits",
        ],
      },
      {
        kind: "ownership",
        metric: "git.code_lines",
        dimension: "repository",
        title: "Ownership concentration — share of lines by top contributors",
      },
      {
        kind: "event-histogram",
        metric: "git.pr_cycle_time_h",
        title: "How long pull requests stayed open",
      },
    ],
  },
  Elements: SCREEN_GAP("Element-level (file/module) analytics"),
};

/* ── Collaboration (ported unchanged from ModalityView configs) ──────── */

const COLLAB: Record<string, LensEntry> = {
  Overview: {
    title: "Collaboration",
    sections: [
      {
        kind: "headline",
        metrics: [
          "collab.messages_sent",
          "collab.meeting_hours",
          "collab.focus_time_pct",
        ],
      },
      {
        kind: "trend",
        metrics: ["collab.messages_sent", "collab.meeting_hours"],
      },
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
        metrics: [
          "collab.messages_sent",
          "collab.msgs_per_active_day",
          "collab.active_days",
        ],
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
      {
        kind: "concentration",
        metrics: ["collab.messages_sent"],
        framing: "load-balance",
      },
    ],
  },
  Meetings: {
    title: "Meetings",
    sections: [
      {
        kind: "headline",
        metrics: [
          "collab.meeting_hours",
          "collab.meetings_count",
          "collab.meeting_free_days",
        ],
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
      {
        kind: "concentration",
        metrics: ["collab.meeting_hours"],
        framing: "load-balance",
      },
    ],
  },
  // emails_received deliberately omitted: distribution-list/CI noise (see git history).
  Email: {
    title: "Email",
    sections: [
      {
        kind: "headline",
        metrics: ["collab.emails_sent", "collab.emails_read"],
      },
      { kind: "trend", metrics: ["collab.emails_sent"] },
      {
        kind: "distribution",
        metric: "collab.emails_sent",
        title: "How many emails people sent",
        caption:
          "How many people fall in each sent-email band — a long right tail means a few people send most of the email.",
        unitLabel: "emails sent per person",
      },
      {
        kind: "concentration",
        metrics: ["collab.emails_sent"],
        framing: "load-balance",
      },
    ],
  },
  "Focus time": {
    title: "Focus time",
    sections: [
      {
        kind: "headline",
        metrics: ["collab.focus_time_pct", "collab.meeting_free_days"],
      },
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
        metrics: [
          "collab.files_shared",
          "collab.files_engaged",
          "collab.files_shared_external",
        ],
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
      {
        kind: "concentration",
        metrics: ["collab.files_shared"],
        framing: "load-balance",
      },
    ],
  },
};

/* ── Knowledge / Wiki ────────────────────────────────────────────────── */

const WIKI: Record<string, LensEntry> = {
  Overview: {
    title: "Knowledge / Wiki",
    sections: [
      {
        kind: "headline",
        metrics: ["wiki.pages_created", "wiki.edits", "wiki.comments"],
      },
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
      {
        kind: "headline",
        metrics: ["wiki.pages_created", "wiki.pages_edited"],
      },
      {
        kind: "distribution",
        metric: "wiki.pages_created",
        title: "How many pages people created",
        caption:
          "Each bar is a range of pages created, and how many people fall in it.",
        unitLabel: "pages created per person",
      },
      {
        kind: "concentration",
        metrics: ["wiki.pages_created"],
        framing: "bus-factor",
      },
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
        caption:
          "Each bar is a range of wiki edits, and how many people fall in it.",
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
};
const SUPPORT_NOTE: LensRoadmap = {
  comingSoon: "Zendesk data is not available yet.",
};

const SALES: Record<string, LensEntry> = Object.fromEntries(
  ["Pipeline", "Deal flow", "Activity", "Velocity & quality"].map((l) => [
    l,
    SALES_NOTE,
  ])
);
const SUPPORT: Record<string, LensEntry> = Object.fromEntries(
  ["Tickets", "CSAT", "Knowledge base", "Comments & updates"].map((l) => [
    l,
    SUPPORT_NOTE,
  ])
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
    // ComingSoon entries contribute nothing; tenant lenses fetch their own
    // tenant-entity collection and must stay out of the person grid.
    if ("comingSoon" in entry || "entity" in entry) continue;
    for (const k of sectionMetricKeys(entry)) keys.add(k);
  }
  return [...keys];
}
