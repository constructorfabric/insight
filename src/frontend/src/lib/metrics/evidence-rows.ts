import type {
  MetricEvidenceColumn,
  MetricEvidenceRow,
  MetricEvidenceSort,
} from "@/api/metric-drilldown-client";
import { formatMetricNumber } from "@/lib/format";

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

export function summaryLine(text: string): string {
  return text.split("\n").find((line) => line.trim() !== "") ?? text;
}

// INVARIANT: `ref` is a PR or issue number, unique only within a repository —
// keying on it alone would join two rows' expansion.
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

/**
 * The order a header click asks for next: ascending, then descending, then
 * back to whatever the server orders by when asked for nothing.
 */
export function nextSort(
  current: MetricEvidenceSort | null,
  key: string
): MetricEvidenceSort | null {
  if (current?.key !== key) return { key, direction: "asc" };
  if (current.direction === "asc") return { key, direction: "desc" };
  return null;
}
