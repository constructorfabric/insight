import { useMemo, useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";

import { AnalyticsApiError } from "@/api/analytics-client";
import {
  MAX_EVIDENCE_PERSONS,
  personsEvidenceSelection,
  queryMetricDrilldown,
  type MetricEvidenceSort,
} from "@/api/metric-drilldown-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import { MetricEvidenceTable } from "@/components/metric-evidence-table";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatMetricNumber } from "@/lib/format";
import { nextSort } from "@/lib/metrics/evidence-rows";
import { sameEvidenceSubject } from "@/lib/metrics/evidence-placeholder";
import { type BucketBreakdownRow } from "@/lib/portal/trend-drilldown";

const PAGE_LIMIT = 100;
/** Where scrolling stops growing the table, as in the metric evidence dialog. */
const MAX_PAGES = 50;

export interface TrendDrilldownState {
  /** Catalog key, or null for a card derived from another metric's rows. */
  metricKey: string | null;
  label: string;
  bucketLabel: string;
  period: { from: string; to: string };
  members: readonly { person_id: string; name: string }[];
  breakdown: BucketBreakdownRow[];
}

export function TrendDrilldownDialog({
  state,
  onClose,
}: {
  state: TrendDrilldownState | null;
  onClose: () => void;
}) {
  return (
    <Dialog open={state != null} onOpenChange={(open) => !open && onClose()}>
      {state ? (
        <DialogContent className="flex h-[calc(100dvh-4rem)] max-h-[52rem] w-[calc(100vw-2rem)] max-w-none flex-col gap-0 overflow-hidden p-0 sm:max-w-[76rem] [&_[data-slot=dialog-close]]:top-5">
          <DialogHeader className="shrink-0 border-b p-5 pr-14">
            <DialogTitle>{state.label}</DialogTitle>
            <p className="text-muted-foreground text-sm">
              {state.members.length}{" "}
              {state.members.length === 1 ? "person" : "people"} ·{" "}
              {state.period.from} – {state.period.to}
            </p>
          </DialogHeader>
          <Body state={state} />
        </DialogContent>
      ) : null}
    </Dialog>
  );
}

function Body({ state }: { state: TrendDrilldownState }) {
  // A derived card has no catalog metric to evidence, so the periods it is
  // built from are the whole story it can tell.
  if (!state.metricKey) {
    return (
      <div className="min-h-0 flex-1 overflow-auto p-5">
        <PeriodTable state={state} />
      </div>
    );
  }

  return (
    <Tabs defaultValue="records" className="flex min-h-0 flex-1 flex-col">
      <TabsList className="mx-5 mt-4 self-start">
        <TabsTrigger value="records">Records</TabsTrigger>
        <TabsTrigger value="periods">By {state.bucketLabel}</TabsTrigger>
      </TabsList>
      <TabsContent value="records" className="min-h-0 flex-1 overflow-hidden">
        <Records metricKey={state.metricKey} state={state} />
      </TabsContent>
      <TabsContent value="periods" className="min-h-0 flex-1 overflow-auto p-5">
        <PeriodTable state={state} />
      </TabsContent>
    </Tabs>
  );
}

/**
 * The team's records, in one read.
 *
 * The roster is the entity: the catalog defines these metrics for a person,
 * but the drilldown accepts a group of them, so the question "what is this
 * team total made of" is one request that the server orders, narrows and pages
 * — not a request per member joined afterwards, which could only ever order
 * what it had already fetched.
 */
function Records({
  metricKey,
  state,
}: {
  metricKey: string;
  state: TrendDrilldownState;
}) {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const [sort, setSort] = useState<MetricEvidenceSort | null>(null);

  const selection = useMemo(
    () =>
      personsEvidenceSelection(
        { metric_key: metricKey, period: state.period, filters: [] },
        state.members.map((member) => member.person_id),
        state.period,
      ),
    [metricKey, state.members, state.period],
  );
  const view = useMemo(() => (sort ? { sort } : {}), [sort]);
  const queryKey = ["metric-drilldown", "trend", sessionScope, selection, view];
  const query = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam, signal }) => {
      if (!selection) throw new Error("Metric evidence selection is missing");
      return queryMetricDrilldown(
        { ...selection, ...view, cursor: pageParam, limit: PAGE_LIMIT },
        signal,
      );
    },
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    // Re-ordering re-reads rows already on screen; without this the table
    // blanks to a spinner on every header click. A different chart, member set
    // or period is a different question, and its answer waits for the spinner
    // rather than borrowing the last one's rows.
    placeholderData: (previous, previousQuery) =>
      sameEvidenceSubject(previousQuery?.queryKey, queryKey)
        ? previous
        : undefined,
    enabled: sessionScope != null && selection != null,
    // A busy drilldown answers 429, and asking again immediately is what made
    // it busy.
    retry: (failureCount, error) =>
      failureCount < 1 &&
      (!(error instanceof AnalyticsApiError) || error.status >= 500),
  });

  const rows = useMemo(
    () => query.data?.pages.flatMap((page) => page.rows) ?? [],
    [query.data],
  );
  const columns = query.data?.pages[0]?.columns ?? [];
  // The order of the rows on screen, not the one just asked for; see the
  // records dialog.
  const shownSort = query.data?.pages[0]?.selection?.sort ?? null;
  const pageLimitReached =
    (query.data?.pages.length ?? 0) >= MAX_PAGES && query.hasNextPage;

  // `personsEvidenceSelection` refuses two different scopes. A scope with
  // nobody in it has no records, and telling its reader to narrow it is the
  // opposite of the truth.
  if (!state.members.length) {
    return (
      <p className="text-muted-foreground p-5 text-sm">
        No records in this window.
      </p>
    );
  }

  if (!selection) {
    return (
      <p className="text-muted-foreground p-5 text-sm">
        This scope holds more than {MAX_EVIDENCE_PERSONS} people, which is more
        than one table can stand behind. Narrow it and open the chart again.
      </p>
    );
  }

  if (query.isPending) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }

  // "No records" is a claim about the data. Only make it when the data was
  // actually read.
  if (query.isError && !query.data) {
    return (
      <p className="text-muted-foreground p-5 text-sm" role="alert">
        These records could not be read, so nothing is claimed here. Try again
        in a moment.
      </p>
    );
  }

  if (!rows.length) {
    return (
      <p className="text-muted-foreground p-5 text-sm">
        No records in this window.
      </p>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <MetricEvidenceTable
        metricKey={metricKey}
        rows={rows}
        columns={columns}
        sort={shownSort}
        onSortChange={(key) => setSort((current) => nextSort(current, key))}
        fetchNextPage={query.fetchNextPage}
        hasNextPage={query.hasNextPage && !pageLimitReached}
        isFetchingNextPage={query.isFetchingNextPage}
        reordering={query.isFetching && !query.isFetchingNextPage}
        nextPageError={query.isFetchNextPageError}
        pageLimitReached={pageLimitReached}
      />
    </div>
  );
}

function PeriodTable({ state }: { state: TrendDrilldownState }) {
  if (!state.breakdown.length) {
    return (
      <p className="text-muted-foreground text-sm">
        No readings in this window.
      </p>
    );
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          {/* INVARIANT: the fixed columns must not wrap. `table-layout` is auto
            here, so the unbounded "Who" column can squeeze them below their
            content width — and a date breaks at its own hyphens, landing
            "2026-" above "08-26". */}
          <TableHead className="w-36 whitespace-nowrap capitalize">
            {state.bucketLabel}
          </TableHead>
          <TableHead className="w-28 text-right whitespace-nowrap">
            Total
          </TableHead>
          <TableHead className="w-28 text-right whitespace-nowrap">
            Active
          </TableHead>
          <TableHead>Who</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {state.breakdown.map((row) => (
          <TableRow key={row.date}>
            <TableCell className="whitespace-nowrap tabular-nums">
              {row.date}
            </TableCell>
            <TableCell className="text-right whitespace-nowrap tabular-nums">
              {formatMetricNumber(row.total, "decimal")}
            </TableCell>
            <TableCell className="text-right whitespace-nowrap tabular-nums">
              {row.contributors.length}
            </TableCell>
            <TableCell className="text-muted-foreground">
              {row.contributors.length ? row.contributors.join(", ") : "—"}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
