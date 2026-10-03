/**
 * What a stored metric's definition says about whether an alert can read it,
 * before it is run: a check reads exactly one row and one number.
 */

import type { MetricDefinition } from "@/api/custom-types";

type MetricField = MetricDefinition["fields"][number];

export type MetricShape =
  | { kind: "one" }
  /** One row per group: `by` names what the rows differ by. */
  | { kind: "grouped"; by: string[] }
  /** Raw rows, because a column is neither aggregated nor computed. */
  | { kind: "rows" };

function aggregated(field: MetricField): boolean {
  return field.agg !== undefined || field.divide !== undefined;
}

export function metricShape(definition: MetricDefinition): MetricShape {
  if (definition.limit === 1) return { kind: "one" };

  const groupBy = definition.group_by ?? [];
  if (groupBy.length > 0) return { kind: "grouped", by: groupBy };
  if (!definition.fields.every(aggregated)) return { kind: "rows" };

  return { kind: "one" };
}

const ALWAYS_NUMERIC = new Set(["count", "sum", "avg"]);
const NUMERIC_TYPES = new Set(["int", "float"]);

/**
 * Whether a column holds a number: the same rule the service applies when it
 * types a metric's columns. A count, sum, average or ratio always does; a
 * minimum or maximum does when what it reads is a number.
 */
export function isNumericColumn(field: MetricField): boolean {
  if (field.divide !== undefined) return true;
  if (field.agg === undefined) return NUMERIC_TYPES.has(field.type);
  if (ALWAYS_NUMERIC.has(field.agg)) return true;

  return NUMERIC_TYPES.has(field.type);
}

/** The columns an alert can compare: numbers, and not what the rows are grouped by. */
export function alertColumns(definition: MetricDefinition): MetricField[] {
  const groupBy = new Set(definition.group_by ?? []);

  return definition.fields.filter(
    (field) => !groupBy.has(field.as_name) && isNumericColumn(field)
  );
}

/** Whether the column is a ratio, which is empty when there is nothing to divide by. */
export function isRatio(
  definition: MetricDefinition | undefined,
  column: string
): boolean {
  return (
    definition?.fields.some(
      (field) => field.as_name === column && field.divide !== undefined
    ) ?? false
  );
}
