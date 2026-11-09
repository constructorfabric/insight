import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { MetricDirection, MetricFormat } from "@/api/metric-results-client";
import { metricDisplayUnit } from "@/lib/format";
import {
  personDisplayName,
  type PersonNaming,
} from "@/lib/identities/person-display";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { normalizePersonId } from "@/lib/metrics/entity";
import { byFamily, familyName, familyOf } from "@/lib/reports/families";
import type { IdentityPerson } from "@/types/insight";

export interface MetricsRow {
  entityId: string;
  displayName: string;
  depth: number;
  hasReports: boolean;
  expanded: boolean;
}

export interface MetricGroup {
  family: string;
  name: string;
  keys: string[];
}

export type SortDirection = "ascending" | "descending";

function servesData(metric: MetricDefinition): boolean {
  if (metric.schema_status === "error") return false;
  return metric.origin === "custom" || metric.last_observed_date != null;
}

export function metricsViewCatalog(
  definitions: readonly MetricDefinition[],
  isOffered: (metric: MetricDefinition) => boolean,
): MetricDefinition[] {
  const offered = definitions.filter(
    (metric) =>
      metric.entity_type === "person" &&
      metric.is_enabled &&
      servesData(metric) &&
      isOffered(metric),
  );

  return byFamily(offered).flatMap((family) => family.metrics);
}

export function chunkKeys(keys: readonly string[], size: number): string[][] {
  const chunks: string[][] = [];
  for (let i = 0; i < keys.length; i += size) {
    chunks.push(keys.slice(i, i + size));
  }
  return chunks;
}

export function finiteValue(
  result: NormalizedMetricResult,
  entityId: string,
): number | null {
  const { value } = forEntity(result, entityId);
  return value != null && Number.isFinite(value) ? value : null;
}

export function keysWithData(
  keys: readonly string[],
  byKey: ReadonlyMap<string, NormalizedMetricResult>,
  entityIds: readonly string[],
): string[] {
  return keys.filter((key) => {
    const result = byKey.get(key);
    return result != null && entityIds.some((id) => finiteValue(result, id) != null);
  });
}

function byName(a: { displayName: string }, b: { displayName: string }): number {
  return a.displayName.localeCompare(b.displayName);
}

function treeRows(
  node: IdentityPerson,
  depth: number,
  expanded: ReadonlySet<string>,
): MetricsRow[] {
  const entityId = normalizePersonId(node.person_id);
  const hasReports = node.subordinates.length > 0;
  const open = hasReports && expanded.has(entityId);
  const row = {
    entityId,
    displayName: personDisplayName(node),
    depth,
    hasReports,
    expanded: open,
  };
  if (!open) return [row];

  const reports = [...node.subordinates].sort((a, b) =>
    personDisplayName(a).localeCompare(personDisplayName(b)),
  );

  return [row, ...reports.flatMap((report) => treeRows(report, depth + 1, expanded))];
}

export function metricsRows(
  root: IdentityPerson | null,
  people: ReadonlyArray<PersonNaming & { person_id: string }>,
  expanded: ReadonlySet<string>,
): MetricsRow[] {
  if (root) return treeRows(root, 0, expanded);

  return people
    .map((person) => ({
      entityId: normalizePersonId(person.person_id),
      displayName: personDisplayName(person),
      depth: 0,
      hasReports: false,
      expanded: false,
    }))
    .sort(byName);
}

export function sortRows(
  rows: readonly MetricsRow[],
  metricKey: string,
  direction: SortDirection,
  byKey: ReadonlyMap<string, NormalizedMetricResult>,
): MetricsRow[] {
  const result = byKey.get(metricKey);
  const valueOf = (row: MetricsRow) => (result ? finiteValue(result, row.entityId) : null);
  const sign = direction === "ascending" ? 1 : -1;

  return rows
    .map((row) => ({ ...row, depth: 0 }))
    .sort((a, b) => {
      const left = valueOf(a);
      const right = valueOf(b);
      if (left == null) return right == null ? byName(a, b) : 1;
      if (right == null) return -1;
      return (left - right) * sign || byName(a, b);
    });
}

export function metricGroups(keys: readonly string[]): MetricGroup[] {
  const groups: MetricGroup[] = [];
  for (const key of keys) {
    const family = familyOf(key);
    const last = groups.at(-1);
    if (last?.family === family) last.keys.push(key);
    else groups.push({ family, name: familyName(family), keys: [key] });
  }
  return groups;
}

export interface CompareTeam {
  id: string;
  name: string;
  memberIds: string[];
}

function subtreeIds(node: IdentityPerson): string[] {
  return [normalizePersonId(node.person_id), ...node.subordinates.flatMap(subtreeIds)];
}

export function compareTeams(root: IdentityPerson | null): CompareTeam[] {
  if (!root || root.subordinates.length === 0) return [];

  return [
    {
      id: normalizePersonId(root.person_id),
      name: personDisplayName(root),
      memberIds: subtreeIds(root),
    },
    ...root.subordinates.flatMap(compareTeams),
  ];
}

export interface CompareCell {
  best: boolean;
  gap: number | null;
}

export function compareGaps(
  values: ReadonlyArray<number | null>,
  direction: MetricDirection,
): CompareCell[] {
  const present = values.filter((value): value is number => value != null);
  const best = direction === "lower_is_better" ? Math.min(...present) : Math.max(...present);
  const ranked =
    direction !== "neutral" && present.length >= 2 && present.some((value) => value !== best);

  return values.map((value) => {
    if (!ranked || value == null) return { best: false, gap: null };
    if (value === best) return { best: true, gap: null };
    return {
      best: false,
      gap: best === 0 ? null : Math.round(((value - best) / Math.abs(best)) * 100),
    };
  });
}

export function formatGap(gap: number | null): string {
  if (gap == null) return "";
  if (gap === 0) return "≈";
  return `${gap > 0 ? "+" : "−"}${Math.abs(gap)}%`;
}

export function headerUnit(
  label: string,
  format: MetricFormat,
  unit: string | null,
): string | null {
  const shown = metricDisplayUnit(format, unit);
  if (!shown) return null;
  return label.toLocaleLowerCase().includes(shown.toLocaleLowerCase()) ? null : shown;
}

export function rolesById(root: IdentityPerson | null): Map<string, string> {
  const roles = new Map<string, string>();
  const walk = (node: IdentityPerson) => {
    const role = node.job_title?.trim();
    if (role) roles.set(normalizePersonId(node.person_id), role);
    node.subordinates.forEach(walk);
  };
  if (root) walk(root);
  return roles;
}
