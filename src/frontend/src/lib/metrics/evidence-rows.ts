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

/** The first non-blank line, so a leading newline is not read as a blank cell. */
export function summaryLine(text: string): string {
  return text.split("\n").find((line) => line.trim() !== "") ?? text;
}

/**
 * A key per row that survives re-sorting.
 *
 * Not `ref`: it is a PR or issue number, unique only within a repository, so
 * two rows can share one. Evidence exposes no id of its own, leaving the whole
 * value set as the identity — and rows whose values match to the last field get
 * an occurrence suffix, since a duplicate key would join their expansion.
 */
export function evidenceRowKeys(rows: readonly MetricEvidenceRow[]): string[] {
  const seen = new Map<string, number>();
  return rows.map((row) => {
    const signature = JSON.stringify(
      Object.keys(row.values)
        .sort()
        .map((key) => [key, row.values[key]])
    );
    const occurrence = seen.get(signature) ?? 0;
    seen.set(signature, occurrence + 1);
    return occurrence === 0 ? signature : `${signature}#${occurrence}`;
  });
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
