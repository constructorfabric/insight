import type { MetricBucket } from "@/api/metric-results-client";
import { Card, CardContent } from "@/components/ui/card";
import {
  ChartContainer,
  ChartLine,
  ComposedChart,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { bucketNote, smallMultiples } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const MULTIPLE_CONFIG: ChartConfig = { value: { label: "Value" } };

/** A grid of small line charts, one per dimension value, one shared y-axis. */
export function SmallMultiplesSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "small-multiples" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  const r = resolve(sectionNeeds(section, bucket)[0]);
  if (!r) return null;
  const { multiples, max } = smallMultiples(
    tenantData(r).series,
    section.dimension,
    section.top ?? 12
  );
  // A single multiple is a trend wearing a costume.
  if (multiples.length < 2 || max <= 0) return null;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="p-4 pb-1 text-xs text-muted-foreground">
          {[bucketNote(r.timeseries?.bucket ?? bucket), "one shared y-axis"]
            .filter(Boolean)
            .join(" · ")}
        </CardContent>
        <CardContent className="grid grid-cols-2 gap-x-4 gap-y-3 p-4 md:grid-cols-3 lg:grid-cols-4">
          {multiples.map((multiple) => (
            <div key={multiple.value} className="min-w-0">
              <p
                className="truncate text-xs text-muted-foreground"
                title={multiple.label}
              >
                {multiple.label}
              </p>
              <ChartContainer config={MULTIPLE_CONFIG} className="h-16 w-full">
                <ComposedChart
                  data={multiple.points}
                  margin={{ top: 4, right: 0, left: 0, bottom: 0 }}
                >
                  <XAxis dataKey="date" hide />
                  {/* The shared ceiling is the whole point: the same y-axis
                      everywhere, or the grid invites false comparisons. */}
                  <YAxis hide domain={[0, max]} />
                  <ChartLine
                    dataKey="value"
                    stroke="var(--chart-1)"
                    strokeWidth={1.5}
                    dot={false}
                  />
                </ComposedChart>
              </ChartContainer>
            </div>
          ))}
        </CardContent>
      </Card>
    </section>
  );
}
