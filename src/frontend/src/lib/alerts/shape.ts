import type { DatasetDeclaration, MetricDefinition } from "@/api/custom-types";

import type { ColumnKind } from "./condition";

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

const COUNTED = new Set(["count", "sum", "avg"]);

function kindOfType(type: string): ColumnKind {
  switch (type) {
    case "int":
    case "float":
      return "number";
    case "datetime":
      return "date";
    default:
      return "text";
  }
}

// INVARIANT: the same rule the service's `column_kinds` applies when it
// types a condition, so a test the form offers is one the service accepts.
export function columnKinds(
  definition: MetricDefinition,
  declaration?: DatasetDeclaration
): [string, ColumnKind][] {
  return definition.fields.map((field) => {
    if (field.divide !== undefined || COUNTED.has(field.agg ?? "")) {
      return [field.as_name, "number"];
    }

    const declared = declaration?.fields.find(
      (each) => each.name === field.field
    );
    return [field.as_name, kindOfType(declared?.type ?? field.type)];
  });
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
