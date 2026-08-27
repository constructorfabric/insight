import { useMemo, useState } from "react";
import { useQueries } from "@tanstack/react-query";

import { queryMetricDrilldown } from "@/api/metric-drilldown-client";
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
import {
  nextSort,
  visibleEvidenceRows,
  type EvidenceSort,
} from "@/lib/metrics/evidence-rows";
import {
  mergeMemberRecords,
  type BucketBreakdownRow,
} from "@/lib/portal/trend-drilldown";

/** Most records to ask any one person for before the table stops growing. */
const PER_PERSON_LIMIT = 200;

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
 * The team's records, gathered a person at a time.
 *
 * These metrics are defined for a person and not for a tenant, so there is no
 * single request that answers "the whole team's merged pull requests" — one
 * per member, joined here, is the request the catalog actually supports.
 */
function Records({
  metricKey,
  state,
}: {
  metricKey: string;
  state: TrendDrilldownState;
}) {
  const [sort, setSort] = useState<EvidenceSort | null>({
    key: "date",
    direction: "desc",
  });

  const { loading, failed, truncated, merged } = useQueries({
    queries: state.members.map((member) => ({
      queryKey: [
        "metric-drilldown",
        "trend",
        metricKey,
        member.person_id,
        state.period.from,
        state.period.to,
      ],
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        queryMetricDrilldown(
          {
            metric_key: metricKey,
            entity: { type: "person", id: member.person_id },
            period: state.period,
            filters: [],
            display_dimensions: [],
            limit: PER_PERSON_LIMIT,
          },
          signal,
        ),
    })),
    combine: (results) => ({
      loading: results.some((r) => r.isPending),
      failed: results.filter((r) => r.isError).length,
      truncated: results.some((r) => r.data?.next_cursor != null),
      merged: mergeMemberRecords(
        results.flatMap((result, index) => {
          const member = state.members[index];
          if (!result.data || !member) return [];
          return [
            {
              personId: member.person_id,
              name: member.name,
              columns: result.data.columns,
              rows: result.data.rows,
            },
          ];
        }),
      ),
    }),
  });

  const rows = useMemo(
    () =>
      visibleEvidenceRows({
        rows: merged.rows,
        columns: merged.columns,
        search: "",
        sort,
      }),
    [merged, sort],
  );

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Spinner />
      </div>
    );
  }

  // "No records" is a claim about the data. Only make it when the data was
  // actually read: if every request failed, what is true is that nothing
  // could be read.
  if (failed === state.members.length && failed > 0) {
    return (
      <p className="text-muted-foreground p-5 text-sm">
        Nobody in this scope could be read, so nothing is claimed here. Try
        again in a moment.
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
      {failed > 0 ? (
        <p className="text-muted-foreground px-5 pt-3 text-sm">
          {failed} of {state.members.length}{" "}
          {failed === 1 ? "person" : "people"} could not be read; the table
          below is short by their records.
        </p>
      ) : null}
      {truncated ? (
        <p className="text-muted-foreground px-5 pt-3 text-sm">
          Up to {PER_PERSON_LIMIT} records per person are listed; whoever
          passed that in this window has more than the table shows.
        </p>
      ) : null}
      <MetricEvidenceTable
        metricKey={metricKey}
        rows={rows}
        columns={merged.columns}
        sort={sort}
        onSortChange={(key) => setSort((current) => nextSort(current, key))}
        fetchNextPage={() => Promise.resolve()}
        hasNextPage={false}
        isFetchingNextPage={false}
        nextPageError={false}
        pageLimitReached={false}
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
