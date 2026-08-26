import { useMemo, useState } from "react";
import {
  addDays as addCalendarDays,
  differenceInCalendarDays,
  eachDayOfInterval,
  parseISO,
} from "date-fns";

import type {
  UsageDay,
  UsageEvent,
  UsagePage,
  UsagePerson,
  UsageRange,
} from "@/api/usage-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  MAX_DATE_RANGE_DAYS,
  resolveDateRange,
  toISODate,
} from "@/api/period-to-date-range";
import { PeriodSelectorBar } from "@/components/widgets/period-selector-bar";
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
import { FeedbackTable } from "@/components/portal/platform-usage-feedback";
import {
  PersonName,
  TruncatedCell,
  VirtualTable,
} from "@/components/portal/usage-table";
import { useUsageSummary } from "@/queries/usage";
import { formatDate, formatMetricNumber, formatUtcClock } from "@/lib/format";
import { screenLabel } from "@/lib/portal/screen-label";
import { TEXT_FIGURE, TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import type { CustomRange, PeriodValue } from "@/types/insight";

function daysBetween(from: string, to: string): number {
  return differenceInCalendarDays(parseISO(to), parseISO(from));
}

function addDays(day: string, days: number): string {
  return toISODate(addCalendarDays(parseISO(day), days));
}

function utcToday(): string {
  return new Date().toISOString().slice(0, 10);
}

function fillRange(days: UsageDay[], range: UsageRange): UsageDay[] {
  if (!range.since || !range.until) return days;
  const counted = new Map(days.map((d) => [d.day, d]));
  return eachDayOfInterval({
    start: parseISO(range.since),
    end: parseISO(range.until),
  })
    .slice(0, MAX_DATE_RANGE_DAYS)
    .map((date) => {
      const day = toISODate(date);
      return counted.get(day) ?? { day, visits: 0, visitors: 0 };
    });
}

export function PlatformUsage() {
  const [period, setPeriod] = useState<PeriodValue>("month");
  const [customRange, setCustomRange] = useState<CustomRange | null>(null);
  const range = useMemo(() => {
    const resolved = resolveDateRange(period, customRange);
    if (customRange) return { since: resolved.from, until: resolved.to };
    // Slid whole, not stretched, or a 30-day month covers 31. To the UTC day:
    // the reader's own date names a day the server has no rows for yet.
    const shift = daysBetween(resolved.to, utcToday());
    return { since: addDays(resolved.from, shift), until: addDays(resolved.to, shift) };
  }, [period, customRange]);
  // A custom range outranks the period in `resolveDateRange`, so leaving it set
  // makes every preset inert.
  const choosePeriod = (next: PeriodValue) => {
    setPeriod(next);
    setCustomRange(null);
  };

  // The two reads are siblings, not a sequence: nesting the feedback section
  // under the summary's pending branch delayed its request until the summary
  // landed, and hid it entirely when the summary failed.
  return (
    <div className="flex w-full flex-col gap-6 p-6">
      <PeriodSelectorBar
        period={period}
        customRange={customRange}
        onPeriodChange={choosePeriod}
        onRangeChange={setCustomRange}
      />

      <UsageSummary range={range} />
      <FeedbackTable range={range} />
    </div>
  );
}

function UsageSummary({ range }: { range: UsageRange }) {
  const summary = useUsageSummary(range);

  if (summary.isPending) return <CenteredSpinner />;
  if (summary.isError || !summary.data) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="empty" label="Usage could not be loaded" />
      </div>
    );
  }

  const { totals, by_person, by_page, by_event } = summary.data;
  const by_day = fillRange(summary.data.by_day, {
    since: summary.data.since || range.since,
    until: summary.data.until || range.until,
  });

  return (
    <>
      <div className="grid gap-4 sm:grid-cols-3">
        <Kpi label="visits" value={totals.visits} />
        <Kpi label="people" value={totals.visitors} />
        <Kpi label="pages opened" value={totals.page_views} />
      </div>

      <section className="flex flex-col gap-2">
        <h3 className={TEXT_NAME}>Visits per day</h3>
        {by_day.length === 0 ? <Empty /> : <VisitsChart days={by_day} />}
      </section>

      <PeopleTable rows={by_person} />
      <PagesTable rows={by_page} />
      <EventsTable rows={by_event} />
    </>
  );
}

const CHART_CONFIG = {
  visits: { label: "Visits", color: "var(--chart-1)" },
} satisfies ChartConfig;

function dayTick(day: string): string {
  return formatDate(day, "d MMM");
}

function VisitsChart({ days }: { days: UsageDay[] }) {
  return (
    <ChartContainer config={CHART_CONFIG} className="w-full" style={{ height: 160 }}>
      <BarChart data={days} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" vertical={false} />
        <XAxis
          dataKey="day"
          tickFormatter={dayTick}
          tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
          tickLine={false}
          axisLine={false}
          interval="preserveStartEnd"
          minTickGap={16}
        />
        <YAxis
          allowDecimals={false}
          width={28}
          tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
          tickLine={false}
          axisLine={false}
        />
        <ChartTooltip content={<ChartTooltipContent />} />
        <ChartBar dataKey="visits" fill="var(--color-visits)" radius={[2, 2, 0, 0]} />
      </BarChart>
    </ChartContainer>
  );
}

function Kpi({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border p-4">
      <div className={TEXT_FIGURE}>{formatMetricNumber(value, "integer")}</div>
      <div className={TEXT_LABEL}>{label}</div>
    </div>
  );
}

function Empty() {
  return <ComingSoon variant="row" state="empty" label="No usage in this period yet" />;
}

function PeopleTable({ rows }: { rows: UsagePerson[] }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className={TEXT_NAME}>Who opened it</h3>
      {rows.length === 0 ? (
        <Empty />
      ) : (
        <VirtualTable
          label="Who opened it"
          rows={rows}
          rowKey={(row) => row.person_id}
          columns={[
            {
              header: "Person",
              cell: (row) => <PersonName row={row} />,
            },
            { header: "Visits", width: 6, align: "right", cell: (row) => row.visits },
            { header: "Pages", width: 6, align: "right", cell: (row) => row.page_views },
            { header: "Last seen (UTC)", width: 11, cell: (row) => formatUtcClock(row.last_seen, "d MMM HH:mm") },
          ]}
        />
      )}
    </section>
  );
}

function EventsTable({ rows }: { rows: UsageEvent[] }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className={TEXT_NAME}>Drill-downs and other actions, by opens</h3>
      {rows.length === 0 ? (
        <Empty />
      ) : (
        <VirtualTable
          label="Drill-downs and other actions"
          rows={rows}
          rowKey={(row) => `${row.event_name}:${row.target}`}
          columns={[
            { header: "Action", cell: (row) => row.event_name },
            {
              header: "Target",
              cell: (row) => (
                <span className="font-mono text-xs text-muted-foreground">
                  {row.target || "—"}
                </span>
              ),
            },
            { header: "Opens", width: 6, align: "right", cell: (row) => row.opens },
            { header: "People", width: 6, align: "right", cell: (row) => row.people },
          ]}
        />
      )}
    </section>
  );
}

function PagesTable({ rows }: { rows: UsagePage[] }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className={TEXT_NAME}>What they opened</h3>
      {rows.length === 0 ? (
        <Empty />
      ) : (
        <VirtualTable
          label="What they opened"
          rows={rows}
          rowKey={(row) => row.path}
          columns={[
            {
              header: "Page",
              cell: (row) => (
                <TruncatedCell detail={row.path}>
                  {screenLabel(row.path)}
                </TruncatedCell>
              ),
            },
            { header: "Views", width: 6, align: "right", cell: (row) => row.views },
            { header: "People", width: 6, align: "right", cell: (row) => row.visitors },
          ]}
        />
      )}
    </section>
  );
}
