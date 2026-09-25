import {
  CustomApiError,
  type DrilldownColumn,
  type DrilldownRow,
} from "@/api/custom-client";
import type { MetricEvidenceRow } from "@/api/metric-drilldown-client";
import { groupedNumber } from "@/components/custom/chart-format";

const URL_CELL = /^https?:\/\/\S+$/;

/**
 * A row as the table draws it: each number written the way the card writes
 * it, and a cell that is an absolute URL linked, so a row can be followed to
 * the pull request, the issue or the page it counted.
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
      values[column.key] = groupedNumber(value, column.percent ? "%" : "");
    }
    if (typeof value === "string" && URL_CELL.test(value)) {
      links[column.key] = value;
    }
  }
  return { values, links };
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
