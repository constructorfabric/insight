import {
  evidenceSelection,
  personsEvidenceSelection,
} from "@/api/metric-drilldown-client";
import type {
  EvidencePeopleView,
  EvidencePersonRow,
} from "@/components/metric-evidence-context";
import { formatMetricValue } from "@/lib/format";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { entityValues } from "@/lib/portal/metric-stats";

/**
 * The roster's two lookups, named rather than positional: both are
 * `Map<string, string>` over the same keys, and swapping them at a call site
 * would put person ids in the name column and still typecheck.
 */
export interface RosterLookups {
  nameByEntity: ReadonlyMap<string, string>;
  personIdByEntity: ReadonlyMap<string, string>;
}

/**
 * The people behind an aggregate figure — a distribution band, a busiest
 * tenth — with the value each of them contributed.
 *
 * Values come from the same period result the figure was drawn from, so the
 * list cannot disagree with the bar or card that opened it, and no request is
 * made to build it. A metric with no readable evidence still lists its people;
 * only the step into records is missing, because only that step needs a source.
 */
export function peopleEvidenceView(
  r: NormalizedMetricResult,
  entityIds: readonly string[],
  title: string,
  roster: RosterLookups
): EvidencePeopleView {
  const rows: EvidencePersonRow[] = entityValues(r, entityIds).map((entry) => {
    const name = roster.nameByEntity.get(entry.id) ?? entry.id;
    const selection = r.drilldown
      ? evidenceSelection(r.selection, entry.id)
      : null;
    return {
      entityId: entry.id,
      personId: roster.personIdByEntity.get(entry.id) ?? null,
      name,
      value: entry.value,
      // Unit dropped, sign kept: the column is headed with the metric, so a
      // unit in every cell would repeat it ("Commits" over "142 commits") —
      // but a percent's "%" is part of the number, not a unit beside it.
      valueText: formatMetricValue(entry.value, r.format, null),
      target: selection ? { selection, label: `${r.label} · ${name}` } : null,
    };
  });
  // Busiest first — the reader opened a band to see who is in it, and within a
  // band the order that carries information is the value itself. Ties break on
  // name so the list does not reshuffle between renders.
  rows.sort(
    (left, right) =>
      right.value - left.value || left.name.localeCompare(right.name)
  );

  // INVARIANT: over the rows, never the caller's ids — a person the metric has
  // no value for is not in the list, so their records are not behind it either.
  const allRecords = r.drilldown
    ? personsEvidenceSelection(
        r.selection,
        rows.map((row) => row.entityId)
      )
    : null;

  return {
    title,
    metricKey: r.metric_key,
    valueLabel: r.short_label ?? r.label,
    rows,
    allRecords: allRecords
      ? { selection: allRecords, label: `${r.label} · all records` }
      : null,
  };
}
