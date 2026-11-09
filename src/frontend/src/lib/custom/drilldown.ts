import {
  CustomApiError,
  type DrilldownColumn,
  type DrilldownRow,
} from "@/api/custom-client";
import type { MetricEvidenceRow } from "@/api/metric-drilldown-client";

const URL_CELL = /^https?:\/\/\S+$/;

const ROUNDED = new Intl.NumberFormat("en", {
  maximumFractionDigits: 2,
  useGrouping: false,
  signDisplay: "negative",
});

/**
 * A row as the table draws it: a whole number as the record holds it, a
 * fraction to two decimals unless the metric marks its column exact, a percent
 * marked as one, and a cell that is an absolute URL linked, so a row can be
 * followed to the pull request, the issue or the page it counted.
 */
export function drawable(
  row: DrilldownRow,
  columns: readonly DrilldownColumn[]
): MetricEvidenceRow {
  const values: Record<string, unknown> = { ...row.values };
  const links: Record<string, string> = {};
  for (const column of columns) {
    const value = values[column.key];
    if (column.type === "number" && value != null) {
      values[column.key] = drawnNumber(value, column);
    }
    if (typeof value === "string" && URL_CELL.test(value)) {
      links[column.key] = value;
    }
  }
  return { values, links };
}

function drawnNumber(value: unknown, column: DrilldownColumn): unknown {
  const fraction = typeof value === "number" && !Number.isInteger(value);
  const shown = fraction && !column.exact ? ROUNDED.format(value) : value;

  return column.percent ? `${String(shown)}%` : shown;
}

/**
 * Whether a page was refused because the table behind the metric was made
 * again while it was being read. Every cursor issued over the old table is
 * void, so the walk starts from its first page rather than retrying one.
 */
export function startsOver(error: unknown): boolean {
  return (
    error instanceof CustomApiError &&
    JSON.stringify(error.body ?? null).includes("SOURCE_REBUILT")
  );
}
