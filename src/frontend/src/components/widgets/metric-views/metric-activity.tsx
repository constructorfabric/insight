import { useMemo, useState } from "react";

import { evidenceSelection } from "@/api/metric-drilldown-client";
import { useMetricEvidenceOptional } from "@/components/metric-evidence-context";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import { silentDays, stripDays, type StripDay } from "@/lib/insight/day-strip";
import { metricComparisons } from "@/lib/insight/metric-comparison";
import { metricHelp } from "@/lib/insight/metric-help";
import {
  activityEvents,
  dailyReadings,
  finestGrain,
} from "@/lib/insight/metric-grain";
import { formatDate, formatMetricValue } from "@/lib/format";
import {
  forEntity,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";
import { useMetricDetail } from "@/queries/metric-detail";
import { cn } from "@/lib/utils";

/** Events listed inline before the rest goes to the evidence dialog. */
const EVENTS_SHOWN = 8;

/**
 * What a person actually did, at the closest grain the metric offers.
 *
 * A section is where a number is explained, and the explanation a reader
 * wants is the work behind it: which commits, on which days, against what
 * denominator. Each metric declares how closely it can be read, so this
 * renders that and nothing more — a counter is never dressed up as a list of
 * things, and a metric that offers no detail says so rather than being drawn
 * as though it had some.
 */
export function MetricActivity({
  metric,
  previous,
  entityId,
  periodNoun,
}: {
  metric: NormalizedMetricResult;
  previous: NormalizedMetricResult | null;
  entityId: string;
  /** "month" / "week" — what the change is measured against. */
  periodNoun: string;
}) {
  const grain = finestGrain(metric);
  const selection = metric.selection
    ? evidenceSelection(metric.selection, entityId)
    : null;
  const detail = useMetricDetail(selection, grain != null);
  const help = metricHelp(metric);
  const data = forEntity(metric, entityId);
  const total = formatMetricValue(data.value, metric.format, metric.unit);
  const against = metricComparisons(metric, previous, entityId);

  return (
    <section className="flex flex-col gap-2 border-t py-4 first:border-t-0">
      <header className="flex items-baseline justify-between gap-4">
        <div className="min-w-0">
          <MetricName metric={metric} className="text-sm font-medium" />
          {help?.description ? (
            <p className="truncate text-xs text-muted-foreground">
              {help.description}
            </p>
          ) : null}
        </div>
        <div className="shrink-0 text-right">
          <div className="text-sm tabular-nums">{total}</div>
          {/* Both readings, stated and neither judged. The reader's own last
              period comes first because it is the one they can act on; the
              cohort follows, because they did not choose it and cannot see
              who is in it. */}
          {/* No standing mark here, though the list below carries one on
              every row. What makes that mark readable is the shared line the
              list draws behind it: a dot means something against a visible
              middle and nothing at all on its own. Three rows would not earn
              a line of their own either — the mark exists to make fifteen
              rows scannable, and here both comparisons are already in words. */}
          <div className="text-xs text-muted-foreground">
            {[
              against.change
                ? `${against.change} since last ${periodNoun}`
                : null,
              against.median ? `team ${against.median}` : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </div>
        </div>
      </header>
      <Body grain={grain} metric={metric} entityId={entityId} detail={detail} />
    </section>
  );
}

function Body({
  grain,
  metric,
  entityId,
  detail,
}: {
  grain: ReturnType<typeof finestGrain>;
  metric: NormalizedMetricResult;
  entityId: string;
  detail: ReturnType<typeof useMetricDetail>;
}) {
  if (grain == null) {
    // Said plainly rather than left blank: a reader who can open the day of
    // every other metric on the page will otherwise read the silence here as
    // a screen that failed to load.
    return (
      <p className="text-xs text-muted-foreground">
        This metric reports a period total only — no daily or per-item detail is
        available for it.
      </p>
    );
  }
  if (detail.isPending) {
    return (
      <div className="flex h-12 items-center">
        <Spinner className="size-4 text-muted-foreground" />
      </div>
    );
  }
  if (detail.isError) {
    return (
      <p className="text-xs text-muted-foreground">
        The detail behind this number could not be loaded.{" "}
        <button
          type="button"
          className="underline underline-offset-2"
          onClick={() => void detail.refetch()}
        >
          Try again
        </button>
      </p>
    );
  }
  const rows = detail.data?.rows ?? [];
  const columns = detail.data?.columns ?? [];
  if (rows.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        Nothing recorded in this period.
      </p>
    );
  }
  if (grain === "event") {
    return <EventList metric={metric} entityId={entityId} rows={rows} />;
  }
  return <DayStrip metric={metric} rows={rows} columns={columns} />;
}

function EventList({
  metric,
  entityId,
  rows,
}: {
  metric: NormalizedMetricResult;
  entityId: string;
  rows: NonNullable<ReturnType<typeof useMetricDetail>["data"]>["rows"];
}) {
  const evidence = useMetricEvidenceOptional();
  const selection = evidenceSelection(metric.selection, entityId);
  const events = useMemo(() => activityEvents(rows), [rows]);
  const shown = events.slice(0, EVENTS_SHOWN);
  const rest = events.length - shown.length;

  return (
    <div className="flex flex-col gap-1">
      <ul className="flex flex-col">
        {shown.map((event, index) => (
          <li
            key={event.ref ?? `${event.date}-${index}`}
            className="flex items-baseline gap-3 py-1 text-xs"
          >
            <span className="w-14 shrink-0 text-muted-foreground tabular-nums">
              {formatDate(event.date)}
            </span>
            <span className="min-w-0 flex-1 truncate">
              {event.title ?? <span className="text-muted-foreground">—</span>}
            </span>
            {event.context ? (
              <span className="hidden max-w-[14rem] shrink-0 truncate text-muted-foreground sm:inline">
                {event.context}
              </span>
            ) : null}
          </li>
        ))}
      </ul>
      {rest > 0 && evidence && selection ? (
        <Button
          type="button"
          variant="link"
          className="h-auto justify-start p-0 text-xs"
          onClick={() => evidence.openEvidence(selection, metric.label)}
        >
          {rest} more
        </Button>
      ) : null}
    </div>
  );
}

/** A ratio's daily value is a fraction; the metric says what to scale it by. */
function scaled(metric: NormalizedMetricResult, value: number): number {
  return metric.computation === "ratio" ? value * (metric.scale ?? 1) : value;
}

/**
 * One day, in words.
 *
 * "No reading" is spelled out rather than shown as a zero, because the whole
 * point of the gap in the drawing is that those are different — and a reader
 * checking a suspicious blank is exactly who reaches for this.
 */
function dayTitle(metric: NormalizedMetricResult, day: StripDay): string {
  const when = formatDate(day.date, "d MMM");
  if (day.value == null) return `${when} — no reading`;
  const value = formatMetricValue(
    scaled(metric, day.value),
    metric.format,
    metric.unit
  );
  if (day.numerator != null && day.denominator != null) {
    return `${when} — ${value} of ${day.denominator}`;
  }
  return `${when} — ${value}`;
}

function DayStrip({
  metric,
  rows,
  columns,
}: {
  metric: NormalizedMetricResult;
  rows: NonNullable<ReturnType<typeof useMetricDetail>["data"]>["rows"];
  columns: NonNullable<ReturnType<typeof useMetricDetail>["data"]>["columns"];
}) {
  const [hovered, setHovered] = useState<number | null>(null);
  const period = metric.selection?.period;
  const days = useMemo(
    () =>
      period
        ? stripDays(dailyReadings(rows, columns), period.from, period.to)
        : [],
    [rows, columns, period]
  );
  if (days.length === 0) return null;

  const hoveredDay = hovered != null ? (days[hovered] ?? null) : null;
  const silent = silentDays(days);
  // One denominator for the whole period is worth naming: it is the thing a
  // reader argues with when a share looks wrong, and it is invisible in the
  // percentage itself.
  const denominators = new Set(
    days.flatMap((d) => (d.denominator != null ? [d.denominator] : []))
  );
  const constantDenominator =
    denominators.size === 1 ? [...denominators][0] : null;

  return (
    <div className="flex flex-col gap-1.5">
      {/* Hover reads out in the caption below rather than in a tooltip per
          day: a month is thirty-one triggers, and a floating card that covers
          its neighbours is the wrong shape for asking "what was that bar". */}
      <div
        className="flex h-10 items-end gap-px border-b"
        onPointerLeave={() => setHovered(null)}
      >
        {days.map((day, index) => (
          <div
            key={day.date}
            onPointerEnter={() => setHovered(index)}
            className="relative flex h-full flex-1 items-end"
          >
            {day.height == null ? null : (
              <div
                className={cn(
                  "w-full rounded-t-[1px] bg-foreground/35",
                  // A measured zero keeps a hairline: without it the day is
                  // indistinguishable from one the source said nothing about,
                  // and those mean opposite things.
                  day.height === 0 && "bg-foreground/20"
                )}
                style={{ height: `${Math.max(day.height * 100, 2)}%` }}
              />
            )}
          </div>
        ))}
      </div>
      <div className="flex justify-between text-[0.6875rem] text-muted-foreground">
        <span>{period ? formatDate(period.from) : null}</span>
        <span
          className={cn(
            "text-center",
            hoveredDay ? "text-foreground tabular-nums" : null
          )}
        >
          {hoveredDay
            ? dayTitle(metric, hoveredDay)
            : constantDenominator != null
              ? `measured against ${constantDenominator} per day`
              : silent > 0
                ? `${silent} ${silent === 1 ? "day" : "days"} with no reading`
                : null}
        </span>
        <span>{period ? formatDate(period.to) : null}</span>
      </div>
    </div>
  );
}
