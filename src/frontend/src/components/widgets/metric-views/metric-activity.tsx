import { useMemo, useState } from "react";

import { evidenceSelection } from "@/api/metric-drilldown-client";
import {
  useEvidenceScope,
  useMetricEvidenceOptional,
  withOwnTarget,
} from "@/components/metric-evidence-context";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { MetricName } from "@/components/widgets/metric-help-tooltip";
import {
  provisionalDays,
  silentDays,
  stripDays,
  uncollectedDays,
  type StripDay,
} from "@/lib/insight/day-strip";
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
import {
  activityEventLabel,
  evidenceRecordLinks,
  withSourceDimension,
} from "@/lib/metrics/provider-links";
import { RecordLink } from "@/components/record-link";
import { useMetricDetail } from "@/queries/metric-detail";
import {
  useCollectedThrough,
  useDeclaredMetricDimensions,
} from "@/queries/metric-definitions";
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
  const declared = useDeclaredMetricDimensions();
  const base = metric.selection
    ? evidenceSelection(metric.selection, entityId)
    : null;
  // INVARIANT: the same gate the evidence dialog applies — `source` is what
  // makes a link safe, and asking for it where a metric does not declare it is
  // rejected outright, so the read waits for the catalogue.
  const selection = base
    ? withSourceDimension(base, declared.byMetricKey?.get(base.metric_key))
    : null;
  const detail = useMetricDetail(
    selection,
    grain != null && !declared.isPending
  );
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
  const scope = useEvidenceScope();
  const selection = evidenceSelection(metric.selection, entityId);
  const metricKey = metric.selection?.metric_key ?? "";
  const events = useMemo(() => activityEvents(rows), [rows]);
  const shown = events.slice(0, EVENTS_SHOWN);
  const rest = events.length - shown.length;

  return (
    <div className="flex flex-col gap-1">
      <ul className="flex flex-col">
        {shown.map((event, index) => {
          const links = evidenceRecordLinks(metricKey, event.values);
          const label = activityEventLabel(metricKey, event.ref, event.title);
          return (
            <li
              key={event.ref ?? `${event.date}-${index}`}
              className="flex items-baseline gap-3 py-1 text-xs"
            >
              <span className="w-14 shrink-0 text-muted-foreground tabular-nums">
                {formatDate(event.date)}
              </span>
              <span className="min-w-0 flex-1 truncate">
                {label ? (
                  <RecordLink href={links.title}>{label}</RecordLink>
                ) : (
                  <span className="text-muted-foreground">—</span>
                )}
              </span>
              {event.context ? (
                <span className="hidden max-w-[14rem] shrink-0 truncate text-muted-foreground sm:inline">
                  <RecordLink href={links.repository}>
                    {event.context}
                  </RecordLink>
                </span>
              ) : null}
            </li>
          );
        })}
      </ul>
      {rest > 0 && evidence && selection ? (
        <Button
          type="button"
          variant="link"
          className="h-auto justify-start p-0 text-xs"
          onClick={() =>
            evidence.openEvidenceTargets(
              withOwnTarget(scope, { selection, label: metric.label }),
              { activeMetricKey: selection.metric_key }
            )
          }
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
  if (!day.collected) return `${when} — not collected yet`;
  if (day.value == null) return `${when} — no reading`;
  const value = formatMetricValue(
    scaled(metric, day.value),
    metric.format,
    metric.unit
  );
  const suffix = day.provisional ? ", may still change" : "";
  if (day.numerator != null && day.denominator != null) {
    return `${when} — ${value} of ${day.denominator}${suffix}`;
  }
  return `${when} — ${value}${suffix}`;
}

/**
 * The strip in words, for anyone not reading it with their eyes.
 *
 * States the span, the busiest day and how many days hold no reading — the
 * three things the drawing is for. The per-day readout stays a pointer
 * enhancement over this.
 */
function stripSummary(
  metric: NormalizedMetricResult,
  days: StripDay[],
  period: { from: string; to: string } | undefined
): string {
  const span = period
    ? `${formatDate(period.from)} to ${formatDate(period.to)}`
    : `${days.length} days`;
  const silent = silentDays(days);
  const busiest = days.reduce<StripDay | null>(
    (best, day) =>
      day.value != null && (best?.value == null || day.value > best.value)
        ? day
        : best,
    null
  );
  const pending = uncollectedDays(days);
  const parts = [`${metric.label} by day, ${span}`];
  if (busiest?.value != null)
    parts.push(`busiest ${dayTitle(metric, busiest)}`);
  parts.push(
    silent === 0
      ? "every collected day has a reading"
      : `${silent} ${silent === 1 ? "day has" : "days have"} no reading`
  );
  if (pending > 0) {
    parts.push(
      `${pending} ${pending === 1 ? "day is" : "days are"} not collected yet`
    );
  }
  const open = provisionalDays(days);
  if (open > 0) {
    parts.push(
      `${open} ${open === 1 ? "day may" : "days may"} still change`
    );
  }
  return `${parts.join("; ")}.`;
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
  const { collectedThrough, revisionWindowDays } = useCollectedThrough(
    metric.metric_key
  );
  const days = useMemo(
    () =>
      period
        ? stripDays(
            dailyReadings(rows, columns),
            period.from,
            period.to,
            collectedThrough,
            revisionWindowDays
          )
        : [],
    [rows, columns, period, collectedThrough, revisionWindowDays]
  );
  if (days.length === 0) return null;

  const hoveredDay = hovered != null ? (days[hovered] ?? null) : null;
  // Which way the hover readout grows: rightwards from the bar over the first
  // half of the strip, leftwards over the second, so it never runs off an end.
  const leftAnchored = hovered != null && hovered < days.length / 2;
  const silent = silentDays(days);
  const pending = uncollectedDays(days);
  const open = provisionalDays(days);
  // One denominator for the whole period is worth naming: it is the thing a
  // reader argues with when a share looks wrong, and it is invisible in the
  // percentage itself.
  const denominators = new Set(
    days.flatMap((d) => (d.denominator != null ? [d.denominator] : []))
  );
  const constantDenominator =
    denominators.size === 1 ? [...denominators][0] : null;

  return (
    <div className="mt-5 flex flex-col gap-1.5">
      {/* The reading appears over the bar the pointer is on, not in the
          caption below. Put anywhere else it is a change in the middle of a
          chart, which is exactly where a reader looking at one bar does not
          look — they hover, the number moves somewhere in their periphery, and
          they never learn it was there.

          One positioned element rather than a tooltip per day: a month is
          thirty-one triggers, and thirty-one floating cards is both heavy and
          the wrong shape. It sits ABOVE the bars so it never covers the
          neighbours being compared against.

          The strip carries its own description rather than making each day
          focusable. Thirty-one tab stops per strip, three strips to a section,
          is a worse answer for a keyboard reader than one sentence saying what
          the shape is — and the per-day figure is an enhancement over content
          the header and caption already state. */}
      <div
        className="relative flex h-10 items-end gap-px border-b"
        role="img"
        aria-label={stripSummary(metric, days, period)}
        onPointerLeave={() => setHovered(null)}
      >
        {hoveredDay && hovered != null ? (
          <div
            aria-hidden
            className="pointer-events-none absolute bottom-full z-10 mb-1 rounded border bg-popover px-1.5 py-0.5 text-xs whitespace-nowrap text-popover-foreground tabular-nums shadow-sm"
            style={{
              // Anchored to the bar's own edge and grown inwards, rather than
              // centred and clamped. Centring needs to know the readout's
              // width to keep it inside, and it does not: the text varies from
              // "3 messages" to "3.5 hours of 8", so any constant guarding the
              // ends is a guess that the longer strings walk straight past.
              // Growing inwards cannot overflow whatever the text says.
              //
              // Which edge is the bar's own edge depends on the direction it
              // grows in: a readout on the left half starts at the bar's left
              // boundary, one on the right half ENDS at its right boundary.
              // Taking the left boundary for both put every right-half readout
              // a full bar to the left of the bar it describes, and left the
              // last bar with a strip of empty space beside it.
              left: `${((hovered + (leftAnchored ? 0 : 1)) / days.length) * 100}%`,
              transform: leftAnchored ? undefined : "translateX(-100%)",
            }}
          >
            {dayTitle(metric, hoveredDay)}
          </div>
        ) : null}
        {days.map((day, index) => (
          <div
            key={day.date}
            onPointerEnter={() => setHovered(index)}
            className="relative flex h-full flex-1 items-end"
          >
            {!day.collected ? (
              // A wash over the whole column, not a bar: it says the day was
              // never delivered, and at this weight it cannot be misread as a
              // value the way any bottom-anchored height would be.
              <div className="h-full w-full bg-foreground/8" />
            ) : day.height == null ? null : (
              <div
                className={cn(
                  "w-full rounded-t-[1px] bg-foreground/35",
                  // A measured zero keeps a hairline: without it the day is
                  // indistinguishable from one the source said nothing about,
                  // and those mean opposite things.
                  day.height === 0 && "bg-foreground/20",
                  // Still open to revision: the reading is real, so it keeps
                  // its height and fades. Fading rather than re-toning leaves
                  // the zero's own tone intact, so a zero that may still move
                  // stays distinguishable from one that has settled.
                  day.provisional && "opacity-50"
                )}
                style={{ height: `${Math.max(day.height * 100, 2)}%` }}
              />
            )}
          </div>
        ))}
      </div>
      <div className="flex justify-between text-xs text-muted-foreground">
        <span>{period ? formatDate(period.from) : null}</span>
        <span className="text-center">
          {pending > 0
            ? `${pending} ${pending === 1 ? "day" : "days"} not collected yet`
            : open > 0
              ? `last ${open} ${open === 1 ? "day" : "days"} may still change`
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
