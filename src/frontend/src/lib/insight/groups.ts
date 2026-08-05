import type {
  MetricCollectionConfig,
  MetricTimeseriesGroupLimitConfig,
} from "@/lib/metrics/collection";
import type {
  MetricTimeseriesTableColumnConfig,
  MetricTimeseriesTableConfig,
} from "@/lib/metrics/timeseries-table";
import type { MetricTimeseriesChartConfig } from "@/lib/metrics/timeseries-chart";

/**
 * Dashboard composition registry: named groups of metrics and the KPI row.
 * This file is the single place a new metrics-backed group is added — one
 * `MetricGroup` entry (plus a renderer only if it needs a chart kind outside
 * the vocabulary).
 *
 * A group is a named set of metrics with presentation defaults — nothing ties
 * it to a connector or source family; a group may mix metrics from any
 * families (today's groups align with connector classes only because AI is
 * the first migrated family). Meaning (labels, units, formats, directions,
 * explanations) is server-owned and rides the `/v1/metric-results` response;
 * groups carry metric KEYS and presentation choices only. Group titles are
 * dashboard composition, so they live here.
 */

type BreakdownChartKind = "bars" | "summary-card";
type HistogramChartKind = "histogram";

/**
 * One chart in a group's drilldown. Blocks compose: a multi-metric chart
 * is `metrics: [a, b]` (axes config joins when a family needs dual-axis).
 * The supporting grid is not a block — every group metric renders
 * value + peer-comparison below the blocks by convention. Chart kinds are
 * constrained per view so a block can't pair a composition chart with a
 * timeseries payload.
 */
export type DrilldownBlock =
  | {
      id: string;
      view: "timeseries";
      metrics: string[];
      defaultPresentation?: "chart" | "table";
      chart?: MetricTimeseriesChartConfig;
      groupBy?: {
        default: string;
        options?: string[];
        limits?: Record<string, MetricTimeseriesGroupLimitConfig>;
      };
      table?: MetricTimeseriesTableConfig;
    }
  | { view: "breakdown"; chart: BreakdownChartKind; metrics: string[] }
  | { view: "histogram"; chart: HistogramChartKind; metrics: string[] };

export interface MetricGroup {
  id: GroupId;
  title: string;
  collection: MetricCollectionConfig;
  card: { preview: string[] };
  drilldown: DrilldownBlock[];
}

export type GroupId =
  | "task_delivery"
  | "git_output"
  | "collaboration"
  | "ai_adoption"
  | "wiki";

const TASK_DELIVERY_COLLECTION: MetricCollectionConfig = {
  metrics: [
    {
      key: "tasks.closed",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "tasks.bugs_fixed",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "tasks.dev_time",
      views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
    },
    {
      key: "tasks.resolution_time",
      views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
    },
    {
      key: "tasks.pickup_time",
      views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
    },
    {
      key: "tasks.flow_efficiency",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "tasks.reopen_rate", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "tasks.due_date_compliance",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "tasks.on_time_delivery",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "tasks.avg_slip", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "tasks.estimation_accuracy",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "tasks.worklog_accuracy",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "tasks.bugs_ratio", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "tasks.stale_in_progress",
      views: [{ view: "period" }, { view: "peer" }],
    },
  ],
};

const AI_ADOPTION_COLLECTION: MetricCollectionConfig = {
  metrics: [
    {
      key: "ai.accepted_lines",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "ai.removed_lines", views: [{ view: "period" }, { view: "peer" }] },
    { key: "ai.active_days", views: [{ view: "period" }, { view: "peer" }] },
    { key: "ai.cost", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "ai.accepted_edit_actions",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "ai.tool_acceptance_rate",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "ai.assistant_messages",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "ai.assistant_actions",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "ai.dev_conversations",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "ai.chat_assistant_conversations",
      views: [{ view: "period" }, { view: "peer" }],
    },
  ],
};

const GIT_OUTPUT_COLLECTION: MetricCollectionConfig = {
  metrics: [
    {
      key: "git.commits",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "git.prs_merged",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "git.lines_added",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "git.pr_cycle_time_h",
      views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
    },
    {
      key: "git.pr_size",
      views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
    },
    {
      key: "git.commit_size",
      views: [{ view: "period" }, { view: "peer" }, { view: "histogram" }],
    },
    { key: "git.code_lines", views: [{ view: "period" }, { view: "peer" }] },
    { key: "git.prs_created", views: [{ view: "period" }, { view: "peer" }] },
    { key: "git.merge_rate", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "git.commits_per_active_day",
      views: [{ view: "period" }, { view: "peer" }],
    },
  ],
};

const GIT_LINES_TABLE_COLUMN = {
  label: "Lines",
  template: [
    { metric: "git.lines_added", prefix: "+", tone: "success" },
    { text: " / " },
    {
      metric: "git.lines_removed",
      prefix: "−",
      tone: "destructive",
    },
  ],
} satisfies MetricTimeseriesTableColumnConfig;

const COLLABORATION_COLLECTION: MetricCollectionConfig = {
  metrics: [
    {
      key: "collab.messages_sent",
      views: [
        { view: "period" },
        { view: "peer" },
        { view: "breakdown", dimensions: ["tool"] },
      ],
    },
    {
      key: "collab.channel_posts",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "collab.dm_ratio", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "collab.msgs_per_active_day",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.active_days",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.meeting_hours",
      views: [
        { view: "period" },
        { view: "peer" },
        { view: "breakdown", dimensions: ["tool"] },
      ],
    },
    {
      key: "collab.meetings_count",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.meeting_free_days",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.meetings_organized",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.adhoc_meetings",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.scheduled_meetings",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.focus_time_pct",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "collab.breadth", views: [{ view: "period" }, { view: "peer" }] },
    {
      key: "collab.emails_sent",
      views: [
        { view: "period" },
        { view: "peer" },
        // Single-tool today; the summary card hides its breakdown section
        // below two groups, so this lights up if a second mail source lands.
        { view: "breakdown", dimensions: ["tool"] },
      ],
    },
    {
      key: "collab.emails_received",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.emails_read",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.files_engaged",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.files_shared_internal",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.files_shared_external",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "collab.files_shared",
      views: [
        { view: "period" },
        { view: "peer" },
        { view: "breakdown", dimensions: ["scope"] },
      ],
    },
  ],
};

const WIKI_COLLECTION: MetricCollectionConfig = {
  metrics: [
    {
      key: "wiki.pages_created",
      views: [{ view: "period" }, { view: "peer" }],
    },
    {
      key: "wiki.edits",
      views: [{ view: "period" }, { view: "peer" }],
    },
    { key: "wiki.pages_edited", views: [{ view: "period" }, { view: "peer" }] },
    { key: "wiki.comments", views: [{ view: "period" }, { view: "peer" }] },
  ],
};

export const GROUPS: readonly MetricGroup[] = [
  {
    id: "task_delivery",
    title: "Task delivery",
    collection: TASK_DELIVERY_COLLECTION,
    card: {
      preview: ["tasks.closed", "tasks.resolution_time", "tasks.pickup_time"],
    },
    drilldown: [
      {
        id: "task-throughput",
        view: "timeseries",
        metrics: ["tasks.closed", "tasks.bugs_fixed"],
        chart: { multiMetric: "combined" },
      },
      {
        chart: "histogram",
        view: "histogram",
        metrics: ["tasks.resolution_time"],
      },
      {
        chart: "histogram",
        view: "histogram",
        metrics: ["tasks.pickup_time"],
      },
      { chart: "histogram", view: "histogram", metrics: ["tasks.dev_time"] },
    ],
  },
  {
    id: "git_output",
    title: "Git output",
    collection: GIT_OUTPUT_COLLECTION,
    card: {
      preview: ["git.commits", "git.prs_merged", "git.code_lines"],
    },
    drilldown: [
      {
        id: "output-by-repository",
        view: "timeseries",
        metrics: [
          "git.commits",
          "git.prs_merged",
          "git.lines_added",
          "git.lines_removed",
        ],
        defaultPresentation: "table",
        table: {
          columns: [
            { metric: "git.commits" },
            { metric: "git.prs_merged", labelSource: "short" },
            GIT_LINES_TABLE_COLUMN,
          ],
        },
        groupBy: {
          default: "repository",
          limits: {
            repository: {
              count: 10,
              rankBy: "git.commits",
              includeRemainder: true,
            },
          },
        },
      },
      {
        id: "lines-added-by-category",
        view: "timeseries",
        metrics: ["git.lines_added", "git.lines_removed"],
        table: {
          columns: [GIT_LINES_TABLE_COLUMN],
        },
        groupBy: {
          default: "category",
        },
      },
      {
        chart: "histogram",
        view: "histogram",
        metrics: ["git.pr_cycle_time_h"],
      },
      { chart: "histogram", view: "histogram", metrics: ["git.pr_size"] },
      { chart: "histogram", view: "histogram", metrics: ["git.commit_size"] },
    ],
  },
  {
    id: "collaboration",
    title: "Collaboration",
    collection: COLLABORATION_COLLECTION,
    card: {
      preview: [
        "collab.messages_sent",
        "collab.meeting_hours",
        "collab.focus_time_pct",
      ],
    },
    drilldown: [
      // Modality headline cards (period total + dimension breakdown) instead
      // of trend charts — one card per modality. Everything else takes its
      // standing in the peer story below; a second card row echoed the
      // story's outliers.
      {
        chart: "summary-card",
        view: "breakdown",
        metrics: [
          "collab.meeting_hours",
          "collab.messages_sent",
          "collab.emails_sent",
          "collab.files_shared",
        ],
      },
    ],
  },
  {
    id: "ai_adoption",
    title: "AI adoption",
    collection: AI_ADOPTION_COLLECTION,
    card: {
      preview: ["ai.active_days", "ai.accepted_lines", "ai.cost"],
    },
    drilldown: [
      {
        id: "accepted-lines-by-tool",
        view: "timeseries",
        metrics: ["ai.accepted_lines"],
        groupBy: { default: "tool" },
      },
    ],
  },
  {
    id: "wiki",
    title: "Wiki",
    collection: WIKI_COLLECTION,
    card: {
      preview: ["wiki.pages_created", "wiki.edits", "wiki.comments"],
    },
    drilldown: [
      {
        id: "wiki-activity",
        view: "timeseries",
        metrics: ["wiki.pages_created", "wiki.edits"],
        chart: { multiMetric: "combined" },
      },
    ],
  },
];

/**
 * The members heatmap's fixed cross-family column set: one metric key per
 * column, in display order. Every key exists in a group collection above.
 * The FE owns only the key list and order — each column's label, unit,
 * format, and direction ride the `/v1/metric-results` response.
 */
export const HEATMAP_METRIC_KEYS: readonly string[] = [
  "tasks.closed",
  "tasks.resolution_time",
  "tasks.bugs_fixed",
  "git.prs_merged",
  "git.pr_cycle_time_h",
  "collab.focus_time_pct",
  "collab.meeting_hours",
  "ai.active_days",
  "wiki.edits",
];

/**
 * Heatmap fetch collection: `period` (the cell value) + `peer` (each person's
 * standing vs their OWN department cohort). No timeseries/breakdown/histogram,
 * so a large roster chunks under the backend's projected-row limit.
 */
export const HEATMAP_COLLECTION: MetricCollectionConfig = {
  metrics: HEATMAP_METRIC_KEYS.map((key) => ({
    key,
    views: [{ view: "period" }, { view: "peer" }],
  })),
};

/**
 * The "At a glance" KPI row: array order is display order. Tiles are metric
 * keys resolved against the KPI collection below and render through the
 * display-ready tile intermediate — selectors own formatting and scoring.
 */
export const KPI_ROW: readonly string[] = [
  "tasks.closed",
  "collab.focus_time_pct",
  "git.prs_merged",
  "ai.active_days",
  "ai.accepted_lines",
];

export const KPI_ROW_COLLECTION: MetricCollectionConfig = {
  metrics: KPI_ROW.map((key) => ({
    key,
    views: [{ view: "period" }, { view: "peer" }],
  })),
};

/** KPI tiles navigate to the group that owns their metric. */
export function groupIdForMetricKey(metricKey: string): GroupId | null {
  for (const def of GROUPS) {
    if (def.collection.metrics.some((m) => m.key === metricKey)) return def.id;
  }
  return null;
}
