import type { DateRange } from "@/api/period-to-date-range";
import i18n from "@/i18n";
import type { Matrix, MatrixCell } from "@/lib/export/matrix";
import { isDurationUnit, roundMetricValue } from "@/lib/format";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { finiteValue } from "@/lib/portal/metrics-view";

export interface MetricsViewExport {
  people: ReadonlyArray<{ entityId: string; displayName: string }>;
  keys: readonly string[];
  byKey: ReadonlyMap<string, NormalizedMetricResult>;
  roles: ReadonlyMap<string, string>;
  range: DateRange;
}

function fileUnit({ format, unit }: NormalizedMetricResult): string | null {
  if (format === "percent") return "%";
  if (format === "currency") return "$";
  return unit;
}

const WORD_CHAR = /[\p{L}\p{N}]/u;

function isWordChar(char: string | undefined): boolean {
  return char !== undefined && WORD_CHAR.test(char);
}

function namesUnit(label: string, unit: string): boolean {
  const haystack = label.toLowerCase();
  const needle = unit.toLowerCase();
  if (!needle) return false;

  for (let at = haystack.indexOf(needle); at !== -1; at = haystack.indexOf(needle, at + 1)) {
    if (!isWordChar(haystack[at - 1]) && !isWordChar(haystack[at + needle.length])) return true;
  }
  return false;
}

function columnTitle(key: string, result: NormalizedMetricResult | undefined): string {
  const label = result?.label ?? key;
  const unit = result ? fileUnit(result) : null;
  return unit && !namesUnit(label, unit) ? `${label} (${unit})` : label;
}

function fileValue(
  result: NormalizedMetricResult | undefined,
  entityId: string,
): number | null {
  if (!result) return null;

  const value = finiteValue(result, entityId);
  if (value == null) return null;

  return isDurationUnit(result.unit)
    ? Math.round(value * 100) / 100
    : roundMetricValue(value, result.format);
}

export function metricsViewMatrix({
  people,
  keys,
  byKey,
  roles,
  range,
}: MetricsViewExport): Matrix {
  const withRoles = people.some((person) => roles.has(person.entityId));
  const period = `${range.from} – ${range.to}`;

  const columns = [
    i18n.t("common.table.person"),
    ...(withRoles ? [i18n.t("common.table.role")] : []),
    i18n.t("common.table.period"),
    ...keys.map((key) => columnTitle(key, byKey.get(key))),
  ];
  const rows = people.map((person): MatrixCell[] => [
    person.displayName,
    ...(withRoles ? [roles.get(person.entityId) ?? null] : []),
    period,
    ...keys.map((key) => fileValue(byKey.get(key), person.entityId)),
  ]);

  return { columns, rows };
}

export function metricsViewFilename(range: DateRange, extension: "csv" | "xlsx"): string {
  return `metrics-view_${range.from}_${range.to}.${extension}`;
}
