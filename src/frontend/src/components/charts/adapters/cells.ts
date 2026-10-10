import type { MetricResult } from "@/api/custom-client";

export const OTHER_LABEL = "Other";
export const OTHER_KEY = "other";
const NONE_LABEL = "(none)";

export function columnReader(result: MetricResult, column: string) {
  const at = result.columns.indexOf(column);

  return (row: unknown[]): unknown => (at === -1 ? undefined : row[at]);
}

export function labelOf(cell: unknown): string {
  return cell === null || cell === undefined || cell === ""
    ? NONE_LABEL
    : String(cell);
}

export function largest<T>(
  items: T[],
  size: (item: T) => number,
  count: number
): Set<T> {
  const ranked = items
    .map((item) => [item, size(item)] as const)
    .sort((a, b) => b[1] - a[1]);

  return new Set(ranked.slice(0, count).map(([item]) => item));
}
