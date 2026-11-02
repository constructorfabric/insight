import type { MetricDefinition } from "@/api/custom-client";

/** What a metric reads: its dataset, or its table named in full. */
export function metricSource(definition: MetricDefinition): {
  label: string;
  named: string;
} {
  if (definition.dataset !== undefined) {
    return { label: "Dataset", named: definition.dataset };
  }
  const table = definition.table ?? "";
  const named =
    definition.database && !table.includes(".")
      ? `${definition.database}.${table}`
      : table;

  return { label: "Table", named };
}
