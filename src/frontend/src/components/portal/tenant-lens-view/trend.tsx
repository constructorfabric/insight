import type { MetricBucket } from "@/api/metric-results-client";
import {
  SectionTrend,
  type SectionTrendPoint,
  type SectionTrendSeries,
} from "@/components/portal/section-trend";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { bucketNote, mean, trailingOutlierDates } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/** Buckets of the single org series. */
export function TrendSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "trend" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  const needs = sectionNeeds(section, bucket);
  const series: SectionTrendSeries[] = [];
  const byDate = new Map<string, SectionTrendPoint>();
  const firstMetricPoints: Array<{ date: string; value: number }> = [];
  let servedBucket = bucket;
  for (const [index, key] of section.metrics.entries()) {
    const r = resolve(needs[index]);
    if (!r) continue;
    servedBucket = r.timeseries?.bucket ?? servedBucket;
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
      if (index === 0) {
        firstMetricPoints.push({ date: point.bucket_start, value: point.value });
      }
    }
  }
  const points = [...byDate.values()].sort((a, b) =>
    a.date < b.date ? -1 : 1
  );
  // One bucket draws as a dot pretending to be a trend — say nothing instead.
  if (!series.length || points.length < 2) return null;

  firstMetricPoints.sort((a, b) => (a.date < b.date ? -1 : 1));
  const windowMean = section.referenceMean
    ? mean(firstMetricPoints.map((p) => p.value))
    : null;
  const flagged = section.flagOutliers
    ? trailingOutlierDates(firstMetricPoints)
    : [];

  return (
    <div className="flex flex-col gap-1">
      <SectionTrend
        title={section.title}
        description={[section.description, bucketNote(servedBucket)]
          .filter(Boolean)
          .join(" · ")}
        series={series}
        data={points}
        targetLine={
          windowMean != null
            ? { value: windowMean, label: "window mean" }
            : undefined
        }
      />
      {flagged.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          Fell 2σ below their trailing mean: {flagged.join(", ")}
        </p>
      ) : null}
    </div>
  );
}
