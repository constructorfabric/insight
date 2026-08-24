import {
  SectionTrend,
  type SectionTrendPoint,
  type SectionTrendSeries,
} from "@/components/portal/section-trend";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { tenantData } from "./data";

/** Buckets of the single org series. */
export function TrendSection({
  section,
  data,
}: {
  section: Extract<TenantSectionSpec, { kind: "trend" }>;
  data: Map<string, NormalizedMetricResult>;
}) {
  const series: SectionTrendSeries[] = [];
  const byDate = new Map<string, SectionTrendPoint>();
  for (const key of section.metrics) {
    const r = data.get(key);
    if (!r) continue;
    series.push({
      key,
      label: r.short_label ?? r.label,
      type: section.plot ?? "line",
    });
    for (const point of tenantData(r).series[0]?.points ?? []) {
      if (point.value == null) continue;
      const row = byDate.get(point.bucket_start) ?? {
        date: point.bucket_start,
      };
      row[key] = point.value;
      byDate.set(point.bucket_start, row);
    }
  }
  const points = [...byDate.values()].sort((a, b) =>
    a.date < b.date ? -1 : 1
  );
  // One bucket draws as a dot pretending to be a trend — say nothing instead.
  if (!series.length || points.length < 2) return null;

  return (
    <SectionTrend
      title={section.title}
      description={section.description}
      series={series}
      data={points}
    />
  );
}
