import { useMemo, useState, type ReactNode } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { Search } from "lucide-react";
import {
  addDays as addCalendarDays,
  differenceInCalendarDays,
  eachDayOfInterval,
  parseISO,
} from "date-fns";

import type {
  UsageActionsSort,
  UsageDay,
  UsageList,
  UsagePagesSort,
  UsagePeopleSort,
  UsageRange,
} from "@/api/usage-client";
import { Input } from "@/components/ui/input";
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
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { useUsageOrder } from "@/hooks/use-usage-order";
import { SEARCH_DEBOUNCE_MS } from "@/queries/identity-resolution";
import {
  useUsageActions,
  useUsagePages,
  useUsagePeople,
  useUsageSummary,
} from "@/queries/usage";
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

  // The reads are siblings, not a sequence: a section nested under the
  // summary's pending branch waits for the summary and vanishes when it fails.
  return (
    <div className="flex w-full flex-col gap-6 p-6">
      <PeriodSelectorBar
        period={period}
        customRange={customRange}
        onPeriodChange={choosePeriod}
        onRangeChange={setCustomRange}
      />

      <UsageSummary range={range} />
      <PeopleTable range={range} />
      <PagesTable range={range} />
      <EventsTable range={range} />
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

  const { totals } = summary.data;
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
        <CartesianGrid stroke="var(--grid)" vertical={false} />
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

function Empty({ label = "No usage in this period yet" }: { label?: string }) {
  return <ComingSoon variant="row" state="empty" label={label} />;
}

function ListSection<T>({
  title,
  failure,
  query,
  aside,
  emptyLabel,
  children,
}: {
  title: string;
  failure: string;
  query: UseQueryResult<UsageList<T>>;
  aside?: ReactNode;
  emptyLabel?: string;
  children: (rows: T[], pending: boolean) => ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className={TEXT_NAME}>{title}</h3>
        {aside}
      </div>
      {query.isPending ? (
        <CenteredSpinner />
      ) : query.isError || !query.data ? (
        <ComingSoon variant="row" state="empty" label={failure} />
      ) : query.data.items.length === 0 ? (
        <Empty label={emptyLabel} />
      ) : (
        children(query.data.items, query.isPlaceholderData)
      )}
    </section>
  );
}

function VisitorSearch({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="relative w-full sm:w-56">
      <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder="Search visitors"
        aria-label="Search visitors"
        className="h-8 ps-8"
      />
    </div>
  );
}

function PeopleTable({ range }: { range: UsageRange }) {
  const order = useUsageOrder<UsagePeopleSort>("visits");
  const [typed, setTyped] = useState("");
  const trimmed = typed.trim();
  const debounced = useDebouncedValue(trimmed, SEARCH_DEBOUNCE_MS);
  const search = trimmed === "" ? "" : debounced;
  const people = useUsagePeople(range, order.chosen, search);

  return (
    <ListSection
      title="Who opened it"
      failure="Visitors could not be loaded"
      query={people}
      aside={<VisitorSearch value={typed} onChange={setTyped} />}
      emptyLabel={search ? `Nobody matches “${search}”` : undefined}
    >
      {(rows, pending) => (
        <VirtualTable
          label="Who opened it"
          rows={rows}
          rowKey={(row) => row.person_id}
          order={order.shown}
          onSort={order.toggle}
          pending={pending}
          columns={[
            {
              header: "Person",
              cell: (row) => <PersonName row={row} />,
            },
            { header: "Visits", width: 6, align: "right", sortKey: "visits", cell: (row) => row.visits },
            { header: "Pages", width: 6, align: "right", sortKey: "page_views", cell: (row) => row.page_views },
            {
              header: "Last seen (UTC)",
              width: 11,
              sortKey: "last_seen",
              cell: (row) => formatUtcClock(row.last_seen, "d MMM HH:mm"),
            },
          ]}
        />
      )}
    </ListSection>
  );
}

function EventsTable({ range }: { range: UsageRange }) {
  const order = useUsageOrder<UsageActionsSort>("opens");
  const actions = useUsageActions(range, order.chosen);

  return (
    <ListSection
      title="Drill-downs and other actions"
      failure="Actions could not be loaded"
      query={actions}
    >
      {(rows, pending) => (
        <VirtualTable
          label="Drill-downs and other actions"
          rows={rows}
          rowKey={(row) => `${row.event_name}:${row.target}`}
          order={order.shown}
          onSort={order.toggle}
          pending={pending}
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
            { header: "Opens", width: 6, align: "right", sortKey: "opens", cell: (row) => row.opens },
            { header: "People", width: 6, align: "right", sortKey: "people", cell: (row) => row.people },
          ]}
        />
      )}
    </ListSection>
  );
}

function PagesTable({ range }: { range: UsageRange }) {
  const order = useUsageOrder<UsagePagesSort>("views");
  const pages = useUsagePages(range, order.chosen);

  return (
    <ListSection title="What they opened" failure="Pages could not be loaded" query={pages}>
      {(rows, pending) => (
        <VirtualTable
          label="What they opened"
          rows={rows}
          rowKey={(row) => row.path}
          order={order.shown}
          onSort={order.toggle}
          pending={pending}
          columns={[
            {
              header: "Page",
              cell: (row) => (
                <TruncatedCell detail={row.path}>
                  {screenLabel(row.path)}
                </TruncatedCell>
              ),
            },
            { header: "Views", width: 6, align: "right", sortKey: "views", cell: (row) => row.views },
            { header: "People", width: 6, align: "right", sortKey: "visitors", cell: (row) => row.visitors },
          ]}
        />
      )}
    </ListSection>
  );
}
