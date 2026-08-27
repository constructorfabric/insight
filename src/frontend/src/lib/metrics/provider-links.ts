import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";

const TASK_METRIC_PREFIX = "tasks.";
const GITHUB_STYLE_ISSUE_REF = /^([A-Za-z0-9._-]+\/[A-Za-z0-9._-]+)#([0-9]+)$/;

export function isTaskMetric(metricKey: string): boolean {
  return metricKey.startsWith(TASK_METRIC_PREFIX);
}

export function evidenceRefText(metricKey: string, value: string): string {
  if (!isTaskMetric(metricKey)) return value;
  const matched = GITHUB_STYLE_ISSUE_REF.exec(value.trim());
  return matched ? `#${matched[2]}` : value;
}

export const TYPE_DIMENSION = "type";

export function withTypeDimension(
  selection: MetricEvidenceSelection,
  declared: ReadonlySet<string> | null | undefined
): MetricEvidenceSelection {
  if (!isTaskMetric(selection.metric_key)) return selection;
  return withDimension(selection, TYPE_DIMENSION, declared);
}

function withDimension(
  selection: MetricEvidenceSelection,
  dimension: string,
  declared: ReadonlySet<string> | null | undefined
): MetricEvidenceSelection {
  if (!declared?.has(dimension)) return selection;
  if (selection.display_dimensions.includes(dimension)) return selection;
  return {
    ...selection,
    display_dimensions: [...selection.display_dimensions, dimension].sort(),
  };
}

export function activityEventLabel(
  metricKey: string,
  ref: string | null | undefined,
  title: string | null | undefined
): string | null {
  const shortRef = ref ? evidenceRefText(metricKey, ref) : null;
  if (!isTaskMetric(metricKey)) return title ?? null;
  if (shortRef && title) return `${shortRef}: ${title}`;
  return title ?? shortRef ?? null;
}
