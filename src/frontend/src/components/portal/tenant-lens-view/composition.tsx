import { BarList } from "@/components/portal/domain-lens-view";
import {
  toBarRows,
  UNSPLIT_SEGMENT,
  type BarEntry,
} from "@/lib/portal/bar-rows";
import type { MetricBucket } from "@/api/metric-results-client";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/** The org total cut by one dimension. */
export function CompositionSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "composition" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  // The served rows may be grouped finer than this section's dims (the
  // planner merges summable breakdowns) — the aggregation below re-sums.
  const r = resolve(sectionNeeds(section, bucket)[0]);
  if (!r) return null;
  const entries = new Map<string, BarEntry>();
  for (const row of tenantData(r).breakdown) {
    const dim = row.dimensions.find((d) => d.key === section.dimension);
    if (!dim?.value || row.value == null || row.value <= 0) continue;
    const running = entries.get(dim.value);
    const split = running?.split ?? (section.splitBy ? new Map() : undefined);
    if (split && section.splitBy) {
      const by = row.dimensions.find((d) => d.key === section.splitBy);
      const seed = by?.value || UNSPLIT_SEGMENT;
      const seen = split.get(seed);
      split.set(seed, {
        seed,
        label:
          section.segmentLabels?.[seed] ??
          (by?.label?.trim() || seen?.label || seed),
        value: (seen?.value ?? 0) + row.value,
      });
    }
    entries.set(dim.value, {
      label: dim.label?.trim() || running?.label || dim.value,
      value: (running?.value ?? 0) + row.value,
      split,
    });
  }
  const rows = toBarRows(entries);
  // A single bar with a single segment is an empty shell (rule 11) — but one
  // row CUT by a real split (one environment, several outcomes) is a reading.
  const segments = rows[0]?.segments?.length ?? 0;
  if (rows.length === 0 || (rows.length === 1 && segments < 2)) return null;

  return (
    <BarList
      title={section.title}
      rows={rows}
      format={r.format}
      unit={r.unit}
    />
  );
}
