import type { MetricBucket } from "@/api/metric-results-client";
import { Card, CardContent } from "@/components/ui/card";
import { formatMetricValue } from "@/lib/format";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { decomposeBy } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const SEGMENT_COLORS = ["chart-1", "chart-2", "chart-3", "chart-4", "chart-5"];

/** One 100% bar of a summable metric split by a dimension. */
export function DecompositionSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "decomposition" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  const r = resolve(sectionNeeds(section, bucket)[0]);
  if (!r) return null;
  const segments = decomposeBy(tenantData(r).breakdown, section.splitBy);
  if (segments.length < 2) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-3 p-4">
          <div className="flex h-5 w-full overflow-hidden rounded-[3px]">
            {segments.map((segment, index) => (
              <div
                key={segment.value}
                title={`${section.segmentLabels?.[segment.value] ?? segment.label}: ${segment.share.toFixed(1)}%`}
                style={{
                  width: `${segment.share}%`,
                  background: `var(--${SEGMENT_COLORS[index % SEGMENT_COLORS.length]})`,
                }}
              />
            ))}
          </div>
          <ul className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
            {segments.map((segment, index) => (
              <li key={segment.value} className="flex items-center gap-1.5">
                <span
                  className="size-2 rounded-full"
                  style={{
                    background: `var(--${SEGMENT_COLORS[index % SEGMENT_COLORS.length]})`,
                  }}
                />
                {section.segmentLabels?.[segment.value] ?? segment.label} ·{" "}
                <span className="tabular-nums">
                  {formatMetricValue(segment.amount, r.format, r.unit)} (
                  {segment.share.toFixed(1)}%)
                </span>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>
    </section>
  );
}
