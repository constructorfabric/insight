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
import type { UsagePage, UsagePerson } from "@/api/usage-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { Button } from "@/components/ui/button";
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

  const { totals, by_day, by_person, by_page } = summary.data;
  const busiestDay = by_day.reduce(
    (top, day) => (day.visits > (top?.visits ?? 0) ? day : top),
    by_day[0],
  );

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
        {by_day.length === 0 ? (
          <Empty />
        ) : (
          <div className="flex items-end gap-1" aria-label="Visits per day">
            {by_day.map((day) => (
              <div
                key={day.day}
                title={`${day.day}: ${day.visits} visits, ${day.visitors} people`}
                className="min-h-px w-full rounded-t bg-primary/70"
                style={{
                  height: `${Math.round((day.visits / Math.max(busiestDay?.visits ?? 1, 1)) * 96)}px`,
                }}
              />
            ))}
          </div>
        )}
      </section>

      <PeopleTable rows={by_person} />
      <PagesTable rows={by_page} />
    </div>
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
