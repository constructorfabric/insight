import type {
  MedianMetricResult,
  MetricResult,
  MetricResultsResponse,
  RatioMetricResult,
  SumMetricResult,
} from "@/api/metric-results-client";

/**
 * Canonical wire fixtures for `POST /v1/metric-results`, mirroring the
 * backend's serde wire-shape test
 * (`metric_result_wire_shape_is_flat_with_computation_tag` in
 * `services/analytics/src/domain/metric_results/builder.rs`): the envelope is
 * flat, the `computation` tag carries only executable fields (`scale` on
 * ratio), `unit` is `null` when absent, dimension tuple labels are optional,
 * and every requested view is echoed — empty `values`, never absent views.
 *
 * Single source of truth for the MSW handler and unit tests. If the backend
 * DTO changes shape, update here and nowhere else.
 */

export const SUM_METRIC_FIXTURE: SumMetricResult = {
  metric_key: "ai.accepted_lines",
  label: "Accepted lines",
  description: "Lines of code accepted from AI tools",
  explanation: "Sum of AI-suggested lines the person accepted in the period.",
  unit: "lines",
  format: "integer",
  direction: "higher_is_better",
  computation: "sum",
  views: [
    {
      view: "period",
      values: [
        { entity_id: "alice@example.com", value: 1240 },
        // Period views zero-fill sums for requested entities without rows;
        // "unmeasured" is signaled by the peer view's null target_value.
        { entity_id: "bob@example.com", value: 0 },
      ],
    },
    {
      view: "peer",
      values: [
        {
          entity_id: "alice@example.com",
          target_value: 1240,
          p25: 300,
          median: 800,
          p75: 1500,
          min: 0,
          max: 4200,
          n: 12,
        },
      ],
    },
    {
      view: "timeseries",
      bucket: "day",
      series: [
        {
          entity_id: "alice@example.com",
          dimensions: [
            { key: "tool", value: "claude_code", label: "Claude Code" },
          ],
          points: [
            { bucket_start: "2026-06-01", value: 120 },
            { bucket_start: "2026-06-02", value: null },
            { bucket_start: "2026-06-03", value: 95 },
          ],
        },
        {
          entity_id: "alice@example.com",
          dimensions: [{ key: "tool", value: "cursor" }],
          points: [
            { bucket_start: "2026-06-01", value: 40 },
            { bucket_start: "2026-06-02", value: 12 },
            { bucket_start: "2026-06-03", value: null },
          ],
        },
      ],
    },
    {
      view: "breakdown",
      dimensions: ["tool"],
      values: [
        {
          entity_id: "alice@example.com",
          dimensions: [
            { key: "tool", value: "claude_code", label: "Claude Code" },
          ],
          value: 900,
        },
        {
          entity_id: "alice@example.com",
          dimensions: [{ key: "tool", value: "cursor" }],
          value: 340,
        },
      ],
    },
  ],
};

export const RATIO_METRIC_FIXTURE: RatioMetricResult = {
  metric_key: "ai.tool_acceptance_rate",
  label: "Tool acceptance rate",
  description: "Share of offered AI edits that were accepted",
  unit: null,
  format: "percent",
  direction: "higher_is_better",
  computation: "ratio",
  scale: 100,
  views: [
    {
      view: "period",
      values: [
        { entity_id: "alice@example.com", value: 77.04 },
        { entity_id: "bob@example.com", value: null },
      ],
    },
    {
      view: "peer",
      values: [
        {
          entity_id: "alice@example.com",
          target_value: 77.04,
          p25: 55.2,
          median: 68.9,
          p75: 81.3,
          min: 12.5,
          max: 96.1,
          n: 12,
        },
        {
          entity_id: "bob@example.com",
          target_value: null,
          p25: null,
          median: null,
          p75: null,
          min: null,
          max: null,
          n: 0,
        },
      ],
    },
  ],
};

export const MEDIAN_METRIC_FIXTURE: MedianMetricResult = {
  metric_key: "git.pr_cycle_time_h",
  label: "PR cycle time",
  description: "Hours from a pull request's first commit to merge",
  explanation:
    "Median hours between a merged PR's first commit and its merge, over the person's PRs in the period.",
  unit: "hours",
  format: "decimal",
  direction: "lower_is_better",
  computation: "median",
  views: [
    {
      view: "period",
      values: [
        { entity_id: "alice@example.com", value: 18.5 },
        // Medians are never zero-filled: an unmeasured entity is null.
        { entity_id: "bob@example.com", value: null },
      ],
    },
    {
      view: "peer",
      values: [
        {
          entity_id: "alice@example.com",
          target_value: 18.5,
          p25: 9.2,
          median: 22.4,
          p75: 41.7,
          min: 1.5,
          max: 96.0,
          n: 11,
        },
        {
          entity_id: "bob@example.com",
          target_value: null,
          p25: null,
          median: null,
          p75: null,
          min: null,
          max: null,
          n: 0,
        },
      ],
    },
    {
      view: "histogram",
      values: [
        {
          entity_id: "alice@example.com",
          bins: [
            { lo: 1.5, hi: 20.4, count: 6 },
            { lo: 20.4, hi: 39.3, count: 3 },
            { lo: 39.3, hi: 58.2, count: 2 },
            { lo: 58.2, hi: 77.1, count: 0 },
            { lo: 77.1, hi: 96.0, count: 1 },
          ],
        },
      ],
    },
  ],
};

export const METRIC_RESULTS_RESPONSE_FIXTURE: MetricResultsResponse = {
  metrics: [SUM_METRIC_FIXTURE, RATIO_METRIC_FIXTURE, MEDIAN_METRIC_FIXTURE],
};

/**
 * Collaboration metric metadata for the MSW factory. Only meta is consumed
 * (the factory strips `views` and synthesizes them per request), so `views`
 * here is a minimal placeholder. Covers the collab metrics whose
 * format/direction/computation differ from the factory's sum/integer/
 * higher-is-better default; the rest synthesize correctly without a fixture.
 * Kept out of `METRIC_RESULTS_RESPONSE_FIXTURE` so that canonical response
 * fixture stays stable.
 */
export const COLLAB_METRIC_FIXTURES: MetricResult[] = [
  {
    metric_key: "collab.messages_sent",
    label: "Messages Sent",
    unit: "messages",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 320 }] },
    ],
  },
  {
    metric_key: "collab.meeting_hours",
    label: "Meeting Hours",
    unit: "h",
    format: "decimal",
    direction: "lower_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 18.5 }] },
    ],
  },
  {
    metric_key: "collab.dm_ratio",
    label: "DM Ratio",
    unit: "%",
    format: "percent",
    direction: "lower_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 28.4 }] },
    ],
  },
  {
    metric_key: "collab.msgs_per_active_day",
    label: "Messages per Active Day",
    unit: "messages/day",
    format: "decimal",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 1,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 16 }] },
    ],
  },
  {
    metric_key: "collab.focus_time_pct",
    label: "Focus Time",
    unit: "%",
    format: "percent",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 62.5 }] },
    ],
  },
  {
    metric_key: "collab.active_days",
    label: "Active Days",
    unit: "days",
    format: "integer",
    direction: "higher_is_better",
    computation: "distinct_count",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 17 }] },
    ],
  },
  {
    metric_key: "collab.files_shared",
    label: "Files Shared",
    unit: "files",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 22 }] },
    ],
  },
  {
    metric_key: "collab.breadth",
    label: "Collaboration Breadth",
    unit: "modalities",
    format: "integer",
    direction: "neutral",
    computation: "distinct_count",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 3 }] },
    ],
  },
];

/**
 * Task-delivery metric metadata for the MSW factory. Only meta is consumed
 * (the factory strips `views` and synthesizes them per request), so `views`
 * here is a minimal placeholder. Covers the task metrics whose
 * format/direction/computation differ from the factory's sum/integer/
 * higher-is-better default; the rest synthesize correctly without a fixture.
 * Kept out of `METRIC_RESULTS_RESPONSE_FIXTURE` so that canonical response
 * fixture stays stable.
 */
export const TASK_METRIC_FIXTURES: MetricResult[] = [
  {
    metric_key: "tasks.dev_time",
    label: "Dev Time",
    unit: "h",
    format: "decimal",
    direction: "lower_is_better",
    computation: "median",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 14.2 }] },
    ],
  },
  {
    metric_key: "tasks.resolution_time",
    label: "Resolution Time",
    unit: "d",
    format: "decimal",
    direction: "lower_is_better",
    computation: "median",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 3.5 }] },
    ],
  },
  {
    metric_key: "tasks.pickup_time",
    label: "Pickup Time",
    unit: "d",
    format: "decimal",
    direction: "lower_is_better",
    computation: "median",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 1.2 }] },
    ],
  },
  {
    metric_key: "tasks.flow_efficiency",
    label: "Flow Efficiency",
    unit: "%",
    format: "percent",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 54.0 }] },
    ],
  },
  {
    metric_key: "tasks.reopen_rate",
    label: "Reopen Rate",
    unit: "%",
    format: "percent",
    direction: "lower_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 6.5 }] },
    ],
  },
  {
    metric_key: "tasks.due_date_compliance",
    label: "Due Date Compliance",
    unit: "%",
    format: "percent",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 88.0 }] },
    ],
  },
  {
    metric_key: "tasks.on_time_delivery",
    label: "On-Time Delivery",
    unit: "%",
    format: "percent",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 91.5 }] },
    ],
  },
  {
    metric_key: "tasks.avg_slip",
    label: "Average Slip",
    unit: "d",
    format: "decimal",
    direction: "lower_is_better",
    computation: "ratio",
    scale: 1,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 0.8 }] },
    ],
  },
  {
    metric_key: "tasks.estimation_accuracy",
    label: "Estimation Accuracy",
    unit: "%",
    format: "percent",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 82.0 }] },
    ],
  },
  {
    metric_key: "tasks.worklog_accuracy",
    label: "Worklog Accuracy",
    unit: "%",
    format: "percent",
    direction: "higher_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 95.0 }] },
    ],
  },
  {
    metric_key: "tasks.bugs_ratio",
    label: "Bugs Ratio",
    unit: "%",
    format: "percent",
    direction: "lower_is_better",
    computation: "ratio",
    scale: 100,
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 12.0 }] },
    ],
  },
  {
    metric_key: "tasks.stale_in_progress",
    label: "Stale In Progress",
    unit: "tasks",
    format: "integer",
    direction: "lower_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "alice@example.com", value: 4 }] },
    ],
  },
];

export function metricResultFixtureByKey(key: string): MetricResult | null {
  return (
    METRIC_RESULTS_RESPONSE_FIXTURE.metrics.find((m) => m.metric_key === key) ??
    COLLAB_METRIC_FIXTURES.find((m) => m.metric_key === key) ??
    TASK_METRIC_FIXTURES.find((m) => m.metric_key === key) ??
    null
  );
}
