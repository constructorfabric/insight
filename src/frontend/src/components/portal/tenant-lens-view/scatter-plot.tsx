import type { MetricBucket } from "@/api/metric-results-client";
import { Card, CardContent } from "@/components/ui/card";
import {
  CartesianGrid,
  ChartContainer,
  ChartScatter,
  ChartTooltip,
  ChartTooltipContent,
  ReferenceLine,
  ScatterChart,
  XAxis,
  YAxis,
  ZAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { scatterPoints, type DimRow } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const SCATTER_CONFIG: ChartConfig = { y: { label: "y" } };

/** Two measures per dimension value at once; size is an optional third. */
export function ScatterSection({
  section,
  resolve,
  bucket,
}: {
  section: Extract<TenantSectionSpec, { kind: "scatter" }>;
  resolve: ResolveView;
  bucket: MetricBucket;
}) {
  const needs = sectionNeeds(section, bucket);
  const rx = resolve(needs[0]);
  const ry = resolve(needs[1]);
  const rs = section.size ? resolve(needs[2]) : undefined;
  if (!rx || !ry) return null;
  const rows = (r: NormalizedMetricResult | undefined): DimRow[] =>
    r ? tenantData(r).breakdown : [];
  const { points, medianX, medianY } = scatterPoints(
    rows(rx),
    rows(ry),
    section.size ? rows(rs) : null,
    section.dimension
  );
  // Two points always split cleanly into quadrants — that is geometry, not a
  // reading. Ask for enough dots that position starts to mean something.
  if (points.length < 3) return null;

  const axisLabel = (r: NormalizedMetricResult) =>
    `${r.short_label ?? r.label}${r.unit ? ` (${r.unit})` : ""}`;

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title}
      </p>
      <Card>
        <CardContent className="p-4">
          {section.description ? (
            <p className="mb-3 text-xs text-muted-foreground">
              {section.description}
            </p>
          ) : null}
          <ChartContainer config={SCATTER_CONFIG} className="h-64 w-full">
            <ScatterChart margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
              <CartesianGrid
                strokeDasharray="3 3"
                stroke="var(--border)"
              />
              <XAxis
                type="number"
                dataKey="x"
                name={axisLabel(rx)}
                tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
                tickLine={false}
                axisLine={false}
              />
              <YAxis
                type="number"
                dataKey="y"
                name={axisLabel(ry)}
                tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
                tickLine={false}
                axisLine={false}
              />
              {rs ? <ZAxis type="number" dataKey="size" range={[40, 400]} /> : null}
              {section.quadrants && medianX != null ? (
                <ReferenceLine
                  x={medianX}
                  stroke="var(--muted-foreground)"
                  strokeDasharray="4 4"
                />
              ) : null}
              {section.quadrants && medianY != null ? (
                <ReferenceLine
                  y={medianY}
                  stroke="var(--muted-foreground)"
                  strokeDasharray="4 4"
                />
              ) : null}
              <ChartTooltip
                content={
                  <ChartTooltipContent
                    className="min-w-40"
                    labelFormatter={(_, payload) =>
                      (payload?.[0]?.payload as { label?: string } | undefined)
                        ?.label ?? ""
                    }
                  />
                }
              />
              <ChartScatter
                data={points}
                name={axisLabel(ry)}
                fill="var(--chart-1)"
                fillOpacity={0.7}
              />
            </ScatterChart>
          </ChartContainer>
          <p className="mt-1 text-center text-xs text-muted-foreground">
            {axisLabel(rx)} → across · {axisLabel(ry)} → up
            {section.quadrants ? " · rules at the medians" : ""}
          </p>
        </CardContent>
      </Card>
    </section>
  );
}
