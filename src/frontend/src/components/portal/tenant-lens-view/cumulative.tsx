import type { MetricBucket } from "@/api/metric-results-client";
import { Card, CardContent } from "@/components/ui/card";
import { formatMetricValue } from "@/lib/format";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { cumulativeShares } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const CUMULATIVE_LIMIT = 12;

/** Cumulative share of a summable metric across a ranked dimension. */
export function CumulativeSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "cumulative" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  const r = resolve(sectionNeeds(section, bucket)[0]);
  if (!r) return null;
  const rows = cumulativeShares(tenantData(r).breakdown, section.dimension);
  if (rows.length < 2) return null;
  const shown = rows.slice(0, CUMULATIVE_LIMIT);
  const rest = rows.length - shown.length;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="flex flex-col gap-2 p-4">
          {section.description ? (
            <p className="text-xs text-muted-foreground">{section.description}</p>
          ) : null}
          {shown.map((row) => (
            <div key={row.value} className="flex items-center gap-2 text-xs">
              <span className="w-6 shrink-0 text-right text-muted-foreground">
                {row.rank}
              </span>
              <span
                className="w-40 shrink-0 truncate text-muted-foreground"
                title={row.label}
              >
                {row.label}
              </span>
              <div className="relative h-3 flex-1 overflow-hidden rounded-[2px] bg-muted">
                {/* Running total behind, this row's own share in front. */}
                <div
                  className="absolute inset-y-0 left-0 bg-[var(--chart-2)] opacity-30"
                  style={{ width: `${row.cumulativeShare}%` }}
                />
                <div
                  className="absolute inset-y-0 bg-[var(--chart-1)]"
                  style={{
                    left: `${row.cumulativeShare - row.share}%`,
                    width: `${row.share}%`,
                  }}
                />
              </div>
              <span className="w-32 shrink-0 text-right tabular-nums">
                {formatMetricValue(row.amount, r.format, r.unit)} · Σ{" "}
                {row.cumulativeShare.toFixed(0)}%
              </span>
            </div>
          ))}
          {rest > 0 ? (
            <p className="text-xs text-muted-foreground">
              +{rest} more sharing the remaining{" "}
              {(100 - shown[shown.length - 1].cumulativeShare).toFixed(0)}%
            </p>
          ) : null}
        </CardContent>
      </Card>
    </section>
  );
}
