import type { MetricResult } from "@/api/custom-client";
import { toNumber } from "@/components/custom/chart-format";

export const NONE_LABEL = "(none)";

export function columnReader(result: MetricResult, column: string) {
  const at = result.columns.indexOf(column);

  return (row: unknown[]): unknown => (at === -1 ? undefined : row[at]);
}

export function labelOf(cell: unknown): string {
  return cell === null || cell === undefined || cell === ""
    ? NONE_LABEL
    : String(cell);
}

export { toNumber };
