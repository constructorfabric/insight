import type { MetricBucket } from "@/api/metric-results-client";
import {
  SectionTrend,
  type SectionTrendPoint,
  type SectionTrendSeries,
} from "@/components/portal/section-trend";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { bucketNote, stackedTrend } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/** One metric per bucket, cut by a dimension; `share` draws composition. */
export function StackedTrendSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "stacked-trend" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  const r = resolve(sectionNeeds(section, bucket)[0]);
  if (!r) return null;
  const { segments, rows } = stackedTrend(tenantData(r).series, section.splitBy, {
    share: section.share,
  });
  // One segment stacks into a plain trend, one bucket into a bar — both are
  // other sections' jobs; this one only earns its stack with real data.
  if (segments.length < 2 || rows.length < 2) return null;

  const series: SectionTrendSeries[] = segments.map((segment) => ({
    key: segment.value,
    label: section.segmentLabels?.[segment.value] ?? segment.label,
    type: "stacked-area",
  }));
  const data: SectionTrendPoint[] = rows.map((row) => ({
    date: row.date,
    ...row.values,
  }));
  return (
    <SectionTrend
      title={section.title}
      description={[section.description, bucketNote(r.timeseries?.bucket ?? bucket)]
        .filter(Boolean)
        .join(" · ")}
      series={series}
      data={data}
    />
  );
}
