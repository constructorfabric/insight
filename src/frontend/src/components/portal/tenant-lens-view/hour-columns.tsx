import { Card, CardContent } from "@/components/ui/card";
import {
  BarChart,
  CartesianGrid,
  ChartBar,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  ReferenceArea,
  ReferenceLine,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { hourColumns } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

const COLUMNS_CONFIG: ChartConfig = { value: { label: "Value" } };

/** A rate per two-hour block, with a ±1σ band around the blocks' mean. */
export function HourColumnsSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "hour-columns" }>;
  resolve: ResolveView;
}) {
  // Breakdown needs carry no bucket, so any bucket resolves identically.
  const r = resolve(sectionNeeds(section, "day")[0]);
  if (!r) return null;
  const { columns, mean, stddev } = hourColumns(tenantData(r).breakdown);
  // A band around three columns is a shrug; ask for a real day's spread.
  if (columns.length < 4) return null;

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
          <ChartContainer config={COLUMNS_CONFIG} className="h-56 w-full">
            <BarChart
              data={columns}
              margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
            >
              <CartesianGrid
                vertical={false}
                strokeDasharray="3 3"
                stroke="var(--border)"
              />
              <XAxis
                dataKey="label"
                tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
                tickLine={false}
                axisLine={false}
              />
              <YAxis
                width={28}
                tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
                tickLine={false}
                axisLine={false}
              />
              {mean != null && stddev != null && stddev > 0 ? (
                <ReferenceArea
                  y1={mean - stddev}
                  y2={mean + stddev}
                  fill="var(--chart-2)"
                  fillOpacity={0.12}
                />
              ) : null}
              {mean != null ? (
                <ReferenceLine
                  y={mean}
                  stroke="var(--chart-2)"
                  strokeDasharray="4 4"
                />
              ) : null}
              <ChartTooltip content={<ChartTooltipContent className="min-w-32" />} />
              <ChartBar
                dataKey="value"
                name={r.short_label ?? r.label}
                radius={[2, 2, 0, 0]}
                fill="var(--chart-1)"
              />
            </BarChart>
          </ChartContainer>
        </CardContent>
      </Card>
    </section>
  );
}
