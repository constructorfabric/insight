/**
 * Manage → Platform usage: who opened the product, how often, and what they
 * viewed (#2573).
 *
 * A view-as session records nothing, so no row here is an operator browsing as
 * someone else.
 */

import { useMemo, useState } from "react";
import { useQueries } from "@tanstack/react-query";

import { getPerson } from "@/api/identity-client";
import type {
  UsageDay,
  UsageEvent,
  UsagePage,
  UsagePerson,
  UsageRange,
} from "@/api/usage-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Button } from "@/components/ui/button";
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
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useIsAdmin } from "@/queries/identity-me";
import { useUsageSummary } from "@/queries/usage";
import { cn } from "@/lib/utils";

const PERIODS = [
  { days: 7, label: "7d" },
  { days: 30, label: "30d" },
  { days: 90, label: "90d" },
] as const;

/** Names are resolved for what is on screen, not for the whole ranking. */
const NAMED_ROWS = 25;

/** Every day in [since, until], with the API's counts where it had any. */
function fillRange(days: UsageDay[], range: UsageRange): UsageDay[] {
  if (!range.since || !range.until) return days;
  const counted = new Map(days.map((d) => [d.day, d]));
  const out: UsageDay[] = [];
  const cursor = new Date(`${range.since}T00:00:00Z`);
  const end = new Date(`${range.until}T00:00:00Z`);
  while (cursor <= end && out.length < 400) {
    const day = cursor.toISOString().slice(0, 10);
    out.push(counted.get(day) ?? { day, visits: 0, visitors: 0 });
    cursor.setUTCDate(cursor.getUTCDate() + 1);
  }
  return out;
}

function isoDay(daysBack: number): string {
  const day = new Date();
  day.setUTCDate(day.getUTCDate() - daysBack);
  return day.toISOString().slice(0, 10);
}

export function PlatformUsage() {
  const [days, setDays] = useState<number | null>(30);
  const [custom, setCustom] = useState({ since: isoDay(29), until: isoDay(0) });
  const { isAdmin, isPending: adminPending } = useIsAdmin();
  const range = useMemo(
    () => (days == null ? custom : { since: isoDay(days - 1), until: isoDay(0) }),
    [days, custom],
  );
  const summary = useUsageSummary(range, isAdmin);

  if (adminPending) return <CenteredSpinner />;
  if (!isAdmin) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="empty"
          label="Platform usage is an admin surface"
        />
      </div>
    );
  }
  if (summary.isPending) return <CenteredSpinner />;
  if (summary.isError || !summary.data) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon variant="card" state="empty" label="Usage could not be loaded" />
      </div>
    );
  }

  const { totals, by_person, by_page, by_event } = summary.data;
  // The API returns only days that saw traffic; the chart is about the range,
  // so the quiet days have to be drawn as quiet rather than left out.
  // The window the server actually filtered on, echoed back — the chart covers
  // what the numbers cover, not what the controls asked for a moment ago.
  const by_day = fillRange(summary.data.by_day, {
    since: summary.data.since || range.since,
    until: summary.data.until || range.until,
  });

  return (
    <div className="flex w-full flex-col gap-6 p-6">
      <div className="flex flex-wrap items-center gap-2">
        {PERIODS.map((period) => (
          <Button
            key={period.days}
            size="sm"
            variant={period.days === days ? "default" : "outline"}
            onClick={() => setDays(period.days)}
          >
            {period.label}
          </Button>
        ))}
        <span className="ml-2 text-xs text-muted-foreground">from</span>
        <Input
          type="date"
          aria-label="From"
          className="h-8 w-40"
          value={range.since}
          max={range.until}
          onChange={(e) => {
            setCustom((c) => ({ ...c, since: e.target.value }));
            setDays(null);
          }}
        />
        <span className="text-xs text-muted-foreground">to</span>
        <Input
          type="date"
          aria-label="To"
          className="h-8 w-40"
          value={range.until}
          min={range.since}
          onChange={(e) => {
            setCustom((c) => ({ ...c, until: e.target.value }));
            setDays(null);
          }}
        />
      </div>

      <div className="grid gap-4 sm:grid-cols-3">
        <Kpi label="visits" value={totals.visits} />
        <Kpi label="people" value={totals.visitors} />
        <Kpi label="pages opened" value={totals.page_views} />
      </div>

      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-medium">Visits per day</h3>
        {by_day.length === 0 ? <Empty /> : <VisitsChart days={by_day} />}
      </section>

      <PeopleTable rows={by_person} />
      <PagesTable rows={by_page} />
      <EventsTable rows={by_event} />
    </div>
  );
}

const CHART_CONFIG = {
  visits: { label: "Visits", color: "var(--chart-1)" },
} satisfies ChartConfig;

/** A day reads as `08-14`; the year is already in the range above the chart. */
function dayTick(day: string): string {
  return day.slice(5);
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
      <div className="text-2xl font-semibold">{value.toLocaleString("en-US")}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  );
}

function Empty() {
  return (
    <div className="text-sm text-muted-foreground">No usage in this period yet.</div>
  );
}

function PeopleTable({ rows }: { rows: UsagePerson[] }) {
  const named = rows.slice(0, NAMED_ROWS);
  const profiles = useQueries({
    queries: named.map((row) => ({
      queryKey: ["identity", "person", row.person_id],
      queryFn: () => getPerson(row.person_id),
      staleTime: 5 * 60 * 1000,
      retry: false,
    })),
  });

  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">Who opened it</h3>
      {rows.length === 0 ? (
        <Empty />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Person</TableHead>
              <TableHead className="text-right">Visits</TableHead>
              <TableHead className="text-right">Pages</TableHead>
              <TableHead>Last seen</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row, index) => (
              <TableRow key={row.person_id}>
                <TableCell className={cn("font-medium")}>
                  {profiles[index]?.data?.display_name ?? row.person_id}
                </TableCell>
                <TableCell className="text-right">{row.visits}</TableCell>
                <TableCell className="text-right">{row.page_views}</TableCell>
                <TableCell>{row.last_seen.slice(0, 16)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </section>
  );
}

function EventsTable({ rows }: { rows: UsageEvent[] }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">Drill-downs and other actions, by opens</h3>
      {rows.length === 0 ? (
        <Empty />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Action</TableHead>
              <TableHead>Target</TableHead>
              <TableHead className="text-right">Opens</TableHead>
              <TableHead className="text-right">People</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={`${row.event_name}:${row.target}`}>
                <TableCell className="font-medium">{row.event_name}</TableCell>
                <TableCell className="font-mono text-xs">{row.target || "—"}</TableCell>
                <TableCell className="text-right">{row.opens}</TableCell>
                <TableCell className="text-right">{row.people}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </section>
  );
}

function PagesTable({ rows }: { rows: UsagePage[] }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-medium">What they opened</h3>
      {rows.length === 0 ? (
        <Empty />
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Page</TableHead>
              <TableHead className="text-right">Views</TableHead>
              <TableHead className="text-right">People</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.path}>
                <TableCell className="font-mono text-xs">{row.path}</TableCell>
                <TableCell className="text-right">{row.views}</TableCell>
                <TableCell className="text-right">{row.visitors}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </section>
  );
}
