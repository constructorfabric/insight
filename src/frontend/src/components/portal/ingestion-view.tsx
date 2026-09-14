import { useMemo } from "react";

import type {
  IngestionGrain,
  IngestionIntensity,
  IngestionSeries,
} from "@/api/ingestion-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Button } from "@/components/ui/button";
import {
  BarChart,
  CartesianGrid,
  ChartBar,
  ChartContainer,
  ChartLegend,
  ChartLegendContent,
  ChartTooltip,
  ChartTooltipContent,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
import {
  LOG_FLOOR,
  bandLabel,
  connectorLabel,
  formatUtcBucket,
  formatUtcDay,
  logTicks,
  paddedDomain,
  pivotIntensity,
  scopeForConnector,
  seriesColorVar,
  totalsByKey,
  type PivotRow,
} from "@/lib/ingestion-chart";
import { usePortalSearch, useSetPortalSearch } from "@/lib/portal/portal-search";
import { useIngestionIntensity } from "@/queries/ingestion";
import { formatAxisTick, formatMetricNumber, formatUtcClock } from "@/lib/format";
import { TEXT_BODY, TEXT_LABEL, TEXT_NAME, TEXT_TITLE } from "@/lib/type-scale";

/**
 * How far back the trend reads. Long enough to show a weekly rhythm and a
 * stalled connector, short enough that 15-minute buckets stay one bar each
 * rather than a sub-pixel smear.
 */
const TREND_DAYS = 30;

/** The live close-up re-reads often; the wider windows do not need to. */
const LIVE_REFRESH_MS = 5_000;
const RECENT_REFRESH_MS = 60_000;

/**
 * Admin ops lens over bronze extraction intensity.
 *
 * Two pages, one component: the overview bands every connector, and picking one
 * re-bands the same three charts by the streams inside it. The selected
 * connector lives in the URL (`conn`), so a reload and a shared link both land
 * on the same drill-down.
 */
export function IngestionView() {
  const { conn } = usePortalSearch();
  const setSearch = useSetPortalSearch();
  const scope = conn ? scopeForConnector(conn) : null;

  // `lookbackDays` rather than a resolved `from`: the client turns it into an
  // instant when the request goes out, which keeps the clock out of this render.
  const trend = useIngestionIntensity({
    grain: "15m",
    series: "total",
    scope,
    lookbackDays: TREND_DAYS,
  });
  // Both omit the window on purpose — the server's per-grain default IS the
  // window these charts want (a day back at 15m, 30 minutes at 1s), and an
  // omitted bound keeps the query key stable while the chart refetches.
  const recent = useIngestionIntensity(
    { grain: "15m", scope },
    { refetchInterval: RECENT_REFRESH_MS },
  );
  const live = useIngestionIntensity(
    { grain: "1s", scope },
    { refetchInterval: LIVE_REFRESH_MS },
  );

  return (
    <div className="flex w-full flex-col gap-6 p-6">
      <header className="flex flex-col gap-2">
        {conn ? (
          <div className="flex items-center gap-3">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setSearch({ conn: undefined })}
            >
              ← All connectors
            </Button>
            <h2 className={TEXT_TITLE}>{connectorLabel(conn)}</h2>
          </div>
        ) : (
          <h2 className={TEXT_TITLE}>Ingestion</h2>
        )}
        <p className={`${TEXT_BODY} text-muted-foreground max-w-3xl`}>
          Rows arriving in bronze, bucketed by the timestamp the SOURCE stamped
          at extraction. The destination flushes in batches, so a row can land
          in ClickHouse up to an hour after the bucket it appears in — this is
          extraction intensity, not insert time. Counts are of physical rows and
          are deliberately not deduplicated. All times UTC.
        </p>
      </header>

      <ChartSection
        title={`Extraction trend, last ${TREND_DAYS} days`}
        subtitle="15-minute buckets, log scale, so a single row and a burst are both legible"
        query={trend}
        grain="15m"
        logScale
      />

      <ChartSection
        title={conn ? "Recent activity by stream" : "Recent activity by connector"}
        subtitle="15-minute buckets, stacked"
        query={recent}
        grain="15m"
        stacked
      />

      <ChartSection
        title={conn ? "Live, by stream" : "Live, by connector"}
        subtitle="One-second buckets, stacked"
        query={live}
        grain="1s"
        stacked
      />

      {!conn && <ConnectorRoster query={recent} />}
    </div>
  );
}

interface SectionProps {
  title: string;
  subtitle: string;
  query: ReturnType<typeof useIngestionIntensity>;
  grain: IngestionGrain;
  stacked?: boolean;
  logScale?: boolean;
}

function ChartSection({
  title,
  subtitle,
  query,
  grain,
  stacked,
  logScale,
}: SectionProps) {
  return (
    <section className="flex flex-col gap-2">
      <div className="flex flex-col gap-0.5">
        <h3 className={TEXT_NAME}>{title}</h3>
        <span className={TEXT_LABEL}>{subtitle}</span>
      </div>
      <SectionBody
        query={query}
        grain={grain}
        stacked={stacked}
        logScale={logScale}
      />
    </section>
  );
}

function SectionBody({
  query,
  grain,
  stacked,
  logScale,
}: Omit<SectionProps, "title" | "subtitle">) {
  if (query.isPending) return <CenteredSpinner />;
  if (query.isError || !query.data) {
    return (
      <ComingSoon
        variant="row"
        state="empty"
        label="Ingestion intensity could not be loaded"
      />
    );
  }
  const { points } = query.data;
  if (points.length === 0) {
    return (
      <ComingSoon
        variant="row"
        state="empty"
        label="No rows extracted in this window"
      />
    );
  }
  return (
    <>
      {query.data.truncated && (
        <p className={`${TEXT_LABEL} text-warning`}>
          Clipped: this window produced more buckets than the surface will
          return, so the tail is missing.
        </p>
      )}
      <IntensityChart
        data={query.data}
        grain={grain}
        stacked={stacked === true}
        logScale={logScale === true}
      />
      <WindowNote data={query.data} />
    </>
  );
}

/** The window the server actually read, not the one the chart asked for. */
function WindowNote({ data }: { data: IngestionIntensity }) {
  return (
    <span className={TEXT_LABEL}>
      {formatUtcClock(data.from, "d MMM HH:mm")} →{" "}
      {formatUtcClock(data.to, "d MMM HH:mm")} UTC
    </span>
  );
}

function buildConfig(keys: string[], series: IngestionSeries): ChartConfig {
  return Object.fromEntries(
    keys.map((key) => [
      key,
      { label: bandLabel(key, series), color: seriesColorVar(key) },
    ]),
  );
}

function IntensityChart({
  data,
  grain,
  stacked,
  logScale,
}: {
  data: IngestionIntensity;
  grain: IngestionGrain;
  stacked: boolean;
  logScale: boolean;
}) {
  const { rows, keys } = useMemo(() => pivotIntensity(data.points), [data.points]);
  const config = useMemo(() => buildConfig(keys, data.series), [keys, data.series]);
  // The log axis needs the plotted maximum, which for a stack is the stack
  // height rather than the tallest single band.
  const peak = useMemo(
    () => rows.reduce((max, row) => Math.max(max, row.total), 0),
    [rows],
  );
  const bands = stacked ? keys : ["total"];

  return (
    <ChartContainer config={config} className="w-full" style={{ height: 200 }}>
      <BarChart data={rows} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
        {/* A numeric time axis, not a category one: an hour with no extraction
            must read as a gap, and a categorical axis would close it up. */}
        <XAxis
          dataKey="epoch"
          type="number"
          scale="time"
          domain={paddedDomain(data.from, data.to, grain)}
          tickFormatter={(value: number) =>
            grain === "15m" && rows.length > 200
              ? formatUtcDay(value)
              : formatUtcBucket(value, grain)
          }
          tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
          tickLine={false}
          axisLine={false}
          interval="preserveStartEnd"
          minTickGap={32}
        />
        <YAxis
          {...(logScale
            ? {
                scale: "log" as const,
                // The floor sits below 1 so a single-row bucket still has
                // height; see LOG_FLOOR.
                domain: [LOG_FLOOR, Math.max(peak, 1)],
                ticks: logTicks(peak),
              }
            : { allowDecimals: false })}
          width={36}
          tickFormatter={formatAxisTick}
          tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
          tickLine={false}
          axisLine={false}
        />
        <ChartTooltip
          content={
            <ChartTooltipContent
              labelFormatter={(_label, payload) => {
                const row = payload?.[0]?.payload as PivotRow | undefined;
                return row
                  ? `${formatUtcBucket(row.epoch, grain)} UTC`
                  : "";
              }}
            />
          }
        />
        {stacked && keys.length > 1 && (
          <ChartLegend content={<ChartLegendContent />} />
        )}
        {bands.map((key) => (
          <ChartBar
            key={key}
            dataKey={key}
            stackId={stacked ? "rows" : undefined}
            fill={stacked ? seriesColorVar(key) : "var(--chart-1)"}
            radius={0}
          />
        ))}
      </BarChart>
    </ChartContainer>
  );
}

/**
 * The clickable connector list.
 *
 * Derived from the recent-window read rather than a separate catalogue call, so
 * it lists connectors that extracted something in the window the chart above
 * plots — a connector idle for longer is absent, which the heading says.
 */
function ConnectorRoster({
  query,
}: {
  query: ReturnType<typeof useIngestionIntensity>;
}) {
  const setSearch = useSetPortalSearch();
  const rows = useMemo(
    () => (query.data ? totalsByKey(query.data.points) : []),
    [query.data],
  );

  if (rows.length === 0) return null;

  return (
    <section className="flex flex-col gap-2">
      <h3 className={TEXT_NAME}>Connectors active in the recent window</h3>
      <div className="divide-y rounded-lg border">
        {rows.map((row) => (
          <button
            key={row.key}
            type="button"
            className="flex w-full items-center gap-3 px-4 py-2 text-left hover:bg-muted/50"
            onClick={() => setSearch({ conn: row.key })}
          >
            <span
              aria-hidden
              className="size-2.5 shrink-0 rounded-full"
              style={{ background: seriesColorVar(row.key) }}
            />
            <span className={`${TEXT_BODY} flex-1`}>{row.key}</span>
            <span className={`${TEXT_BODY} tabular-nums text-muted-foreground`}>
              {formatMetricNumber(row.rows, "integer")} rows
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
