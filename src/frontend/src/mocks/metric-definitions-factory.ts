import type { MetricDefinition } from "@/api/metric-definitions-client";
import {
  GROUPS,
  HEATMAP_METRIC_KEYS,
  KPI_ROW,
  type MetricGroup,
} from "@/lib/insight/groups";

import { metaFor } from "./metric-results-factory";

const FAMILY: Record<string, { source: string; dimensions: string[] }> = {
  tasks: { source: "the task tracker", dimensions: ["type", "project"] },
  git: { source: "the git provider", dimensions: ["repository", "language"] },
  collab: { source: "chat, mail and calendar", dimensions: ["surface"] },
  ai: { source: "the coding assistants", dimensions: ["tool", "model"] },
  wiki: { source: "the wiki", dimensions: ["space"] },
};

function familyOf(metricKey: string): string {
  return metricKey.split(".")[0] ?? "";
}

function dimensionsFor(metricKey: string): string[] {
  const seen = new Set<string>();
  for (const group of GROUPS) {
    for (const metric of group.collection.metrics) {
      if (metric.key !== metricKey) continue;
      for (const view of metric.views) {
        if ("dimensions" in view && view.dimensions) {
          for (const d of view.dimensions) seen.add(d);
        }
      }
    }
    for (const block of group.drilldown) {
      if (!("metrics" in block) || !block.metrics.includes(metricKey)) continue;
      if ("groupBy" in block && block.groupBy) {
        seen.add(block.groupBy.default);
        for (const d of Object.keys(block.groupBy.limits ?? {})) seen.add(d);
      }
    }
  }
  if (seen.size) return [...seen];
  return FAMILY[familyOf(metricKey)]?.dimensions ?? [];
}

function keysFrom(group: MetricGroup): string[] {
  return [
    ...group.collection.metrics.map((m) => m.key),
    ...group.card.preview,
    ...group.drilldown.flatMap((b) => ("metrics" in b ? b.metrics : [])),
  ];
}

// A single unavailable key fails the whole /metric-results call, so these come
// from the screens' own config and cannot fall behind it.
function catalogKeys(): string[] {
  return [
    ...new Set([
      ...GROUPS.flatMap(keysFrom),
      ...HEATMAP_METRIC_KEYS,
      ...KPI_ROW,
    ]),
  ].sort();
}

function define(metricKey: string, today: string): MetricDefinition {
  const meta = metaFor(metricKey);
  const family = FAMILY[familyOf(metricKey)];
  const noun = meta.label.toLowerCase();

  return {
    metric_key: metricKey,
    label: meta.label,
    short_label: null,
    description: family
      ? `${meta.label} for the selected period, read from ${family.source}.`
      : `${meta.label} for the selected period.`,
    explanation: `Counted per person and rolled up to whatever scope is in view. Higher is ${
      meta.direction === "higher_is_better" ? "better" : "worse"
    } for ${noun}.`,
    unit: meta.unit,
    format: meta.format,
    direction: meta.direction,
    dimensions: dimensionsFor(metricKey),
    is_enabled: true,
    origin: "builtin",
    schema_status: "ok",
    schema_error_code: null,
    last_observed_date: today,
  };
}

export function buildMetricDefinitions(): MetricDefinition[] {
  const today = new Date().toISOString().slice(0, 10);
  return catalogKeys().map((key) => define(key, today));
}
