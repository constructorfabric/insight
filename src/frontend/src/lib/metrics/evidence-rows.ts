import type {
  MetricEvidenceColumn,
  MetricEvidenceRow,
} from "@/api/metric-drilldown-client";
import { formatMetricNumber } from "@/lib/format";

export type EvidenceSortDirection = "asc" | "desc";

export interface EvidenceSort {
  key: string;
  direction: EvidenceSortDirection;
}

export function cellText(
  value: unknown,
  type: MetricEvidenceColumn["type"]
): string {
  if (value == null) return "—";
  if (type === "number" && typeof value === "number") {
    return formatMetricNumber(value, "decimal");
  }
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

/**
 * The first line of a cell's text, and whether anything followed it.
 *
 * A git commit arrives as its whole message — a subject, a blank line, then a
 * body that runs to dozens of lines. A single-line cell collapses all of it
 * into one run of words and cuts it a few words in, so the cell shows the
 * subject and the detail panel takes the rest. A value whose only remainder is
 * a trailing newline has no body to show.
 */
export function summaryLine(text: string): { line: string; hasMore: boolean } {
  const newline = text.indexOf("\n");
  if (newline === -1) return { line: text, hasMore: false };
  return {
    line: text.slice(0, newline),
    hasMore: text.slice(newline + 1).trim() !== "",
  };
}

/**
 * Identifies a row across re-sorts, so an expanded record stays the one the
 * reader opened rather than whichever record now sits at that position.
 */
export function evidenceRowKey(row: MetricEvidenceRow): string {
  const ref = row.values.ref;
  if (typeof ref === "string" && ref !== "") return ref;
  return JSON.stringify(row.values);
}

function matchesSearch(
  row: MetricEvidenceRow,
  columns: readonly MetricEvidenceColumn[],
  needle: string
): boolean {
  return columns.some((column) =>
    cellText(row.values[column.key], column.type)
      .toLowerCase()
      .includes(needle)
  );
}

function compare(left: unknown, right: unknown): number {
  if (typeof left === "number" && typeof right === "number") {
    return left - right;
  }
  return String(left).localeCompare(String(right), undefined, {
    numeric: true,
  });
}

/**
 * Filter and sort loaded evidence rows.
 *
 * Rows with no value for the sorted column sort last in both directions —
 * a missing field is not a small one, and floating them to the top of an
 * ascending sort would bury the smallest real values.
 */
export function visibleEvidenceRows({
  rows,
  columns,
  search,
  sort,
}: {
  rows: readonly MetricEvidenceRow[];
  columns: readonly MetricEvidenceColumn[];
  search: string;
  sort: EvidenceSort | null;
}): MetricEvidenceRow[] {
  const needle = search.trim().toLowerCase();
  const filtered = needle
    ? rows.filter((row) => matchesSearch(row, columns, needle))
    : [...rows];
  if (!sort) return filtered;

  const factor = sort.direction === "asc" ? 1 : -1;
  return filtered.sort((left, right) => {
    const leftValue = left.values[sort.key];
    const rightValue = right.values[sort.key];
    if (leftValue == null && rightValue == null) return 0;
    if (leftValue == null) return 1;
    if (rightValue == null) return -1;
    return compare(leftValue, rightValue) * factor;
  });
}

export function nextSort(
  current: EvidenceSort | null,
  key: string
): EvidenceSort | null {
  if (current?.key !== key) return { key, direction: "asc" };
  if (current.direction === "asc") return { key, direction: "desc" };
  return null;
}
