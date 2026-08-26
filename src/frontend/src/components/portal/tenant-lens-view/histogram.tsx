import { Card, CardContent } from "@/components/ui/card";
import {
  BarChart,
  CartesianGrid,
  ChartBar,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import { fmtCompact } from "@/lib/portal/metric-stats";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";

import { type ResolveView } from "./plan";
import { tenantData } from "./data";

const HISTOGRAM_CONFIG: ChartConfig = { count: { label: "Runs" } };

/** Per-run distribution served by the backend. */
export function HistogramSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "histogram" }>;
  resolve: ResolveView;
}) {
  const r = resolve({ view: "histogram", metric: section.metric });
  const bins = r ? (tenantData(r).histogram[0]?.bins ?? []) : [];
  if (!r || bins.length === 0) return null;
  const rows = bins.map((bin) => ({
    label: fmtCompact(bin.lo),
    range: `${fmtCompact(bin.lo)}–${fmtCompact(bin.hi)}`,
    count: bin.count,
  }));
  const total = bins.reduce((sum, bin) => sum + bin.count, 0);

  return (
    <section className="flex flex-col gap-3">
      <p className="text-xs font-medium tracking-wider text-muted-foreground uppercase">
        {section.title} · {total} runs
      </p>
      <Card>
        <CardContent className="p-4">
          <p className="mb-3 text-xs text-muted-foreground">{section.caption}</p>
          <ChartContainer config={HISTOGRAM_CONFIG} className="h-56 w-full">
            <BarChart
              data={rows}
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
                interval="preserveStartEnd"
              />
              <YAxis
                allowDecimals={false}
                width={28}
                tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
                tickLine={false}
                axisLine={false}
              />
              <ChartTooltip
                content={
                  <ChartTooltipContent
                    className="min-w-40"
                    labelFormatter={(_, payload) =>
                      (payload?.[0]?.payload as { range?: string } | undefined)
                        ?.range ?? ""
                    }
                  />
                }
              />
              <ChartBar
                dataKey="count"
                name="Runs"
                radius={[2, 2, 0, 0]}
                fill="var(--chart-1)"
              />
            </BarChart>
          </ChartContainer>
          <p className="mt-1 text-center text-xs text-muted-foreground">
            {r.short_label ?? r.label}
            {r.unit ? ` (${r.unit})` : ""}
          </p>
        </CardContent>
      </Card>
    </section>
  );
}
