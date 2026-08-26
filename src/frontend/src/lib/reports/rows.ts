/**
 * What one row of a report is.
 *
 * `people` is the report's original shape: a person per bucket, their org
 * attributes repeated. `repositories` keeps the bucket axis and swaps the
 * other one for a dimension value, summing every person's contribution into
 * it — the roster is still what is asked for, it just stops being the axis.
 */
export type ReportRows = "people" | "repositories";

/** The dimension a row mode groups by, or null when rows are people. */
export function rowDimension(rows: ReportRows): string | null {
  return rows === "repositories" ? "repository" : null;
}

export const ROW_MODES: ReadonlyArray<{ value: ReportRows; label: string }> = [
  { value: "people", label: "People" },
  { value: "repositories", label: "Repositories" },
];
