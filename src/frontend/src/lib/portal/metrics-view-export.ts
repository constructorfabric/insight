import type { DateRange } from "@/api/period-to-date-range";
import type { Matrix, MatrixCell } from "@/lib/export/matrix";
import { isDurationUnit } from "@/lib/format";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import {
  finiteValue,
  headerUnit,
  type MetricsRow,
} from "@/lib/portal/metrics-view";
import { familyName, familyOf } from "@/lib/reports/families";

export type MetricsExportPage = "metrics" | "metrics-compare";

export interface ExportPerson {
  entityId: string;
  displayName: string;
}

interface MetricsExportFrame {
  keys: readonly string[];
  byKey: ReadonlyMap<string, NormalizedMetricResult>;
  roles: ReadonlyMap<string, string>;
  period: DateRange;
}

const TABLE_LEAD = ["Person", "Role", "Period"] as const;
const COMPARE_LEAD = ["Metric", "Period"] as const;

export function metricsExportFilename(
  page: MetricsExportPage,
  period: DateRange,
  extension: "csv" | "xlsx"
): string {
  const from = safeToken(period.from);
  const to = safeToken(period.to);
  return `${page}_${from}_${to}.${extension}`;
}

function safeToken(value: string): string {
  const safe = value.trim().replace(/[^0-9A-Za-z.-]+/g, "");
  return safe || "period";
}

function periodCell(period: DateRange): string {
  return `${period.from} to ${period.to}`;
}

function metricTitle(result: NormalizedMetricResult): string {
  const noted = headerUnit(result.label, result.format, result.unit);
  if (noted) return `${result.label} (${noted})`;

  const suffix = carriedSuffix(result);
  return suffix ? `${result.label} (${suffix})` : result.label;
}

// INVARIANT: the cell is the wire number, so the header keeps the mark the
// figure was already showing. Dropping it leaves 6.47 with no unit.
function carriedSuffix(result: NormalizedMetricResult): string | null {
  if (result.format === "percent" && !result.label.includes("%")) return "%";
  if (result.format === "currency" && !result.label.includes("$")) return "$";
  if (
    result.unit &&
    isDurationUnit(result.unit) &&
    !labelCarriesUnit(result.label, result.unit)
  ) {
    return result.unit;
  }
  return null;
}

function labelCarriesUnit(label: string, unit: string): boolean {
  if (unit === "h") return /\bhours?\b|\bh\b/i.test(label);
  return label.toLocaleLowerCase().includes(unit.toLocaleLowerCase());
}

function countsOf(labels: readonly string[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const label of labels) counts.set(label, (counts.get(label) ?? 0) + 1);
  return counts;
}

function numberDuplicates(labels: readonly string[]): string[] {
  const counts = countsOf(labels);
  const seen = new Map<string, number>();
  return labels.map((label) => {
    if ((counts.get(label) ?? 0) < 2) return label;
    const next = (seen.get(label) ?? 0) + 1;
    seen.set(label, next);
    return next === 1 ? label : `${label} (${next})`;
  });
}

function metricTitles(
  keys: readonly string[],
  byKey: ReadonlyMap<string, NormalizedMetricResult>
): string[] {
  const titles = keys.map((key) => {
    const result = byKey.get(key);
    return result ? metricTitle(result) : key;
  });
  const counts = countsOf(titles);
  const qualified = titles.map((title, index) => {
    const key = keys[index];
    if (!key || (counts.get(title) ?? 0) < 2) return title;
    return `${familyName(familyOf(key))} — ${title}`;
  });
  return numberDuplicates(qualified);
}

function personTitles(
  people: readonly ExportPerson[],
  roles: ReadonlyMap<string, string>
): string[] {
  const titles = people.map((person) => {
    const role = roles.get(person.entityId);
    return role ? `${person.displayName} (${role})` : person.displayName;
  });
  return numberDuplicates(titles);
}

function presentKeys(
  keys: readonly string[],
  byKey: ReadonlyMap<string, NormalizedMetricResult>
): string[] {
  return keys.filter((key) => byKey.has(key));
}

function valueCell(
  result: NormalizedMetricResult | undefined,
  entityId: string
): MatrixCell {
  if (!result) return null;
  return finiteValue(result, entityId);
}

export function metricsTableMatrix(
  input: MetricsExportFrame & { rows: readonly MetricsRow[] }
): Matrix {
  const keys = presentKeys(input.keys, input.byKey);
  const period = periodCell(input.period);

  return {
    columns: [...TABLE_LEAD, ...metricTitles(keys, input.byKey)],
    rows: input.rows.map((row) => [
      row.displayName,
      input.roles.get(row.entityId) ?? "",
      period,
      ...keys.map((key) => valueCell(input.byKey.get(key), row.entityId)),
    ]),
  };
}

export function metricsCompareMatrix(
  input: MetricsExportFrame & { people: readonly ExportPerson[] }
): Matrix {
  const keys = presentKeys(input.keys, input.byKey);
  const titles = metricTitles(keys, input.byKey);
  const people = personTitles(input.people, input.roles);
  const period = periodCell(input.period);

  return {
    columns: [...COMPARE_LEAD, ...people],
    rows: keys.map((key, index) => {
      const result = input.byKey.get(key);
      return [
        titles[index] ?? key,
        period,
        ...input.people.map((person) => valueCell(result, person.entityId)),
      ];
    }),
  };
}
