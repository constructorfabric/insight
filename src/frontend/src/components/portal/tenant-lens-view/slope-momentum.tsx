import { Card, CardContent } from "@/components/ui/card";
import {
  CartesianGrid,
  ChartContainer,
  ChartLine,
  ChartTooltip,
  ChartTooltipContent,
  ComposedChart,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import { formatMetricValue } from "@/lib/format";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import type { TenantSectionSpec } from "@/lib/portal/lens-configs";
import { cn } from "@/lib/utils";

import { halvesComparison, type HalfComparison } from "./derived";
import { sectionNeeds, type ResolveView } from "./plan";
import { tenantData } from "./data";

/** The lines a slope chart stays readable at. */
const SLOPE_LIMIT = 8;
const MOMENTUM_LIMIT = 12;
const STAGE_FIRST = "First half";
const STAGE_SECOND = "Second half";
const LINE_COLORS = ["chart-1", "chart-2", "chart-3", "chart-4", "chart-5"];

function resolveHalves(
  section: Extract<TenantSectionSpec, { kind: "slope" | "momentum" }>,
  resolve: ResolveView
): { r: NormalizedMetricResult; rows: HalfComparison[] } | null {
  // Windowed needs carry no bucket; both halves must have answered.
  const needs = sectionNeeds(section, "day");
  const first = resolve(needs[0]);
  const second = resolve(needs[1]);
  if (!first || !second) return null;
  const rows = halvesComparison(
    tenantData(first).breakdown,
    tenantData(second).breakdown,
    section.dimension
  );
  // A single mover is an anecdote, not momentum.
  return rows.length < 2 ? null : { r: second, rows };
}

function deltaText(r: NormalizedMetricResult, delta: number): string {
  const unit = r.format === "percent" ? " pts" : r.unit ? ` ${r.unit}` : "";
  return `${delta >= 0 ? "+" : "−"}${Math.abs(delta).toFixed(1)}${unit}`;
}

/** First half of the window against the second, one line per value. */
export function SlopeSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "slope" }>;
  resolve: ResolveView;
}) {
  const halves = resolveHalves(section, resolve);
  if (!halves) return null;
  const { r, rows } = halves;
  const top = rows.slice(0, SLOPE_LIMIT);
  // Dimension values carry dots and slashes recharts would read as paths —
  // remap each line to a safe alias (same trick as SectionTrend).
  const keys = top.map((_, index) => `s${index}`);
  const data = [
    {
      stage: STAGE_FIRST,
      ...Object.fromEntries(top.map((row, index) => [keys[index], row.first])),
    },
    {
      stage: STAGE_SECOND,
      ...Object.fromEntries(top.map((row, index) => [keys[index], row.second])),
    },
  ];
  const config: ChartConfig = Object.fromEntries(
    top.map((row, index) => [keys[index], { label: row.label }])
  );

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
          <ChartContainer config={config} className="h-56 w-full">
            <ComposedChart
              data={data}
              margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
            >
              <CartesianGrid
                vertical={false}
                strokeDasharray="3 3"
                stroke="var(--border)"
              />
              <XAxis
                dataKey="stage"
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
              <ChartTooltip content={<ChartTooltipContent className="min-w-40" />} />
              {top.map((row, index) => (
                <ChartLine
                  key={row.value}
                  dataKey={keys[index]}
                  name={row.label}
                  stroke={`var(--${LINE_COLORS[index % LINE_COLORS.length]})`}
                  strokeWidth={2}
                />
              ))}
            </ComposedChart>
          </ChartContainer>
          {/* Direction labelled, not colour-only. */}
          <ul className="mt-2 grid grid-cols-1 gap-x-4 text-xs text-muted-foreground md:grid-cols-2">
            {top.map((row) => (
              <li key={row.value} className="flex justify-between gap-2">
                <span className="truncate" title={row.label}>
                  {row.label}
                </span>
                <span className="whitespace-nowrap tabular-nums">
                  {formatMetricValue(row.first, r.format, r.unit)} →{" "}
                  {formatMetricValue(row.second, r.format, r.unit)} (
                  {deltaText(r, row.delta)})
                </span>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>
    </section>
  );
}

/** Signed change between the window's two halves, one bar per value. */
export function MomentumSection({
  section,
  resolve,
}: {
  section: Extract<TenantSectionSpec, { kind: "momentum" }>;
  resolve: ResolveView;
}) {
  const halves = resolveHalves(section, resolve);
  if (!halves) return null;
  const { r, rows } = halves;
  const top = rows.slice(0, MOMENTUM_LIMIT);
  const maxAbs = Math.max(...top.map((row) => Math.abs(row.delta)), 1e-9);

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
          {top.map((row) => {
            const improving =
              r.direction === "lower_is_better" ? row.delta < 0 : row.delta > 0;
            return (
              <div key={row.value} className="flex items-center gap-2 text-xs">
                <span
                  className="w-40 shrink-0 truncate text-muted-foreground"
                  title={row.label}
                >
                  {row.label}
                </span>
                <div className="relative h-3 flex-1">
                  <div className="absolute inset-y-0 left-1/2 w-px bg-border" />
                  <div
                    className={cn(
                      "absolute inset-y-0 rounded-[2px]",
                      improving ? "bg-[var(--success)]" : "bg-[var(--destructive)]"
                    )}
                    style={{
                      width: `${(Math.abs(row.delta) / maxAbs) * 50}%`,
                      [row.delta >= 0 ? "left" : "right"]: "50%",
                    }}
                  />
                </div>
                <span className="w-16 shrink-0 text-right tabular-nums">
                  {deltaText(r, row.delta)}
                </span>
              </div>
            );
          })}
        </CardContent>
      </Card>
    </section>
  );
}
