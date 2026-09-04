import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  Table,
  TableBody,
  TableCell,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { GearRoadmap } from "@/api/gear-roadmap-client";
import { ShareBar } from "@/components/portal/gear-delivery/parts";
import { NO_METRIC_VALUE, formatMetricNumber } from "@/lib/format";
import { SortableHead } from "@/components/portal/gear-delivery/sortable-head";
import { sortRows, type SortState } from "@/lib/gears/sort";
import { subsystemTone } from "@/lib/gears/subsystem-tone";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import {
  summariseBySubsystem,
  type SubsystemSummary,
} from "@/lib/gears/summary";

export function GearSummary({ roadmap }: { roadmap: GearRoadmap }) {
  const { t } = useTranslation();

  const { openSubsystem } = usePortalNavActions();
  const [sort, setSort] = useState<SortState<SummaryColumn> | null>(null);

  const rows = useMemo(
    () =>
      sortRows(
        summariseBySubsystem(roadmap.gears),
        sort ?? DEFAULT_SUMMARY_SORT,
        summaryValue,
      ),
    [roadmap, sort],
  );


  const totals = rows.reduce(
    (sum, row) => ({
      items: sum.items + row.items,
      done: sum.done + row.done,
      effort: sum.effort + row.effortManDays,
      remaining: sum.remaining + row.remainingManDays,
      unestimated: sum.unestimated + row.unestimated,
    }),
    { items: 0, done: 0, effort: 0, remaining: 0, unestimated: 0 },
  );

  return (
    <section className="flex flex-col gap-3">
      <div className="overflow-x-auto rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/40 hover:bg-muted/40">
              {SUMMARY_COLUMNS.map((column) => (
                <SortableHead
                  key={column.key}
                  column={column.key}
                  label={t(`gear_roadmap.overview.${column.key}`)}
                  sort={sort}
                  onSort={setSort}
                  numeric={column.numeric}
                  className={column.width}
                />
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow
                key={row.subsystem}
                onClick={() => openSubsystem(GEAR_LIST_LENS, row.subsystem)}
                className="cursor-pointer"
                title={t("gear_roadmap.overview.open_subsystem", {
                  subsystem: row.subsystem,
                })}
              >
                <TableCell>
                  <span
                    className={`rounded px-1.5 py-0.5 text-xs font-medium ${
                      subsystemTone(row.subsystem).chip
                    }`}
                  >
                    {row.subsystem}
                  </span>
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.items}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.done}
                </TableCell>
                <TableCell>
                  <ShareBar value={row.donePercent} width="w-16" />
                </TableCell>
                <TableCell>
                  <ShareBar value={row.specReadiness} width="w-16" />
                </TableCell>
                <TableCell>
                  <ShareBar value={row.sdkReadiness} width="w-16" />
                </TableCell>
                <TableCell>
                  <ShareBar value={row.implReadiness} width="w-16" />
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {formatMetricNumber(row.effortManDays, "integer")}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {formatMetricNumber(row.remainingManDays, "integer")}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.unestimated === 0 ? (
                    <span className="text-muted-foreground">{NO_METRIC_VALUE}</span>
                  ) : (
                    row.unestimated
                  )}
                </TableCell>
              </TableRow>
            ))}
            <TableRow className="border-t-2 font-medium hover:bg-transparent">
              <TableCell>{t("gear_roadmap.overview.total")}</TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.items}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.done}
              </TableCell>
              <TableCell colSpan={4} />
              <TableCell className="text-end tabular-nums">
                {formatMetricNumber(totals.effort, "integer")}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {formatMetricNumber(totals.remaining, "integer")}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.unestimated}
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </div>
    </section>
  );
}

/** Where the rollup rests when no column is chosen. */
const DEFAULT_SUMMARY_SORT: SortState<SummaryColumn> = {
  key: "subsystem",
  direction: "asc",
};

/** The lens a subsystem row opens, by the name the registry gives it. */
const GEAR_LIST_LENS = "Gear list";

type SummaryColumn =
  | "subsystem"
  | "items"
  | "done"
  | "done_share"
  | "spec"
  | "sdk"
  | "impl"
  | "effort"
  | "remaining"
  | "unestimated";

const SUMMARY_COLUMNS: {
  key: SummaryColumn;
  numeric?: boolean;
  width?: string;
}[] = [
  { key: "subsystem" },
  { key: "items", numeric: true },
  { key: "done", numeric: true },
  { key: "done_share", width: "w-40" },
  { key: "spec", width: "w-32" },
  { key: "sdk", width: "w-32" },
  { key: "impl", width: "w-32" },
  { key: "effort", numeric: true },
  { key: "remaining", numeric: true },
  { key: "unestimated", numeric: true },
];

function summaryValue(
  row: SubsystemSummary,
  key: SummaryColumn,
): string | number | null {
  switch (key) {
    case "subsystem":
      return row.subsystem;
    case "items":
      return row.items;
    case "done":
      return row.done;
    case "done_share":
      return row.donePercent;
    case "spec":
      return row.specReadiness;
    case "sdk":
      return row.sdkReadiness;
    case "impl":
      return row.implReadiness;
    case "effort":
      return row.effortManDays;
    case "remaining":
      return row.remainingManDays;
    case "unestimated":
      return row.unestimated;
  }
}
