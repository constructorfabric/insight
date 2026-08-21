import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  ArrowDown,
  ArrowUp,
  ChevronDown,
  ChevronRight,
  ChevronsUpDown,
} from "lucide-react";

import type {
  MetricEvidenceColumn,
  MetricEvidenceRow,
} from "@/api/metric-drilldown-client";
import { CopyValueButton } from "@/components/copy-value-button";
import { RecordLink } from "@/components/record-link";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  cellText,
  evidenceRowKeys,
  summaryLine,
  type EvidenceSort,
} from "@/lib/metrics/evidence-rows";
import { evidenceRecordLinks } from "@/lib/metrics/git-links";
import { cn } from "@/lib/utils";

function columnLayout(column: MetricEvidenceColumn) {
  if (column.key === "ref") return { basisRem: 9, grow: 0 };
  if (column.key === "title") return { basisRem: 24, grow: 4 };
  if (column.key === "repository") return { basisRem: 12, grow: 0.5 };
  if (column.key === "author") return { basisRem: 10, grow: 0.25 };
  if (column.key === "date") return { basisRem: 8, grow: 0 };
  if (column.type === "number") return { basisRem: 7, grow: 0 };
  return { basisRem: 9, grow: 1 };
}

const EXPANDER_REM = 2.25;

/**
 * Columns whose link belongs to the summary line only. The full record shows a
 * value entire, and a commit message carries its trailers — one link wrapped
 * around all of that is a paragraph-sized target, not something to click.
 */
const SUMMARY_ONLY_LINKS: ReadonlySet<string> = new Set(["title"]);

function SortIcon({ state }: { state: "asc" | "desc" | null }) {
  if (state === "asc") return <ArrowUp className="size-3.5 shrink-0" />;
  if (state === "desc") return <ArrowDown className="size-3.5 shrink-0" />;
  return (
    <ChevronsUpDown className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/sort:opacity-100" />
  );
}

export function MetricEvidenceTable({
  metricKey,
  rows,
  columns,
  sort,
  onSortChange,
  fetchNextPage,
  hasNextPage,
  isFetchingNextPage,
  nextPageError,
  pageLimitReached,
}: {
  /** Null while the metric picker has yet to resolve one — links stay off. */
  metricKey: string | null;
  rows: MetricEvidenceRow[];
  columns: MetricEvidenceColumn[];
  sort: EvidenceSort | null;
  onSortChange: (key: string) => void;
  fetchNextPage: () => Promise<unknown>;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  nextPageError: boolean;
  pageLimitReached: boolean;
}) {
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const rowKeys = useMemo(() => evidenceRowKeys(rows), [rows]);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => viewport,
    estimateSize: () => 44,
    overscan: 8,
    // INVARIANT: must match the row's React key — the measured height of an
    // expanded row is cached under it.
    getItemKey: useCallback(
      (index: number) => rowKeys[index] ?? index,
      [rowKeys]
    ),
  });
  const virtualRows = virtualizer.getVirtualItems();
  const virtualBodyHeight = virtualizer.getTotalSize();
  const last = virtualRows.at(-1)?.index ?? 0;
  const minimumWidth = columns.reduce((total, column) => {
    return total + columnLayout(column).basisRem;
  }, EXPANDER_REM);

  function toggleRow(key: string): void {
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  }

  useEffect(() => {
    if (
      last >= rows.length - 10 &&
      hasNextPage &&
      !isFetchingNextPage &&
      !nextPageError
    ) {
      void fetchNextPage();
    }
  }, [
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    last,
    nextPageError,
    rows.length,
  ]);

  return (
    <div className="relative min-h-0 flex-1">
      <Table
        role="table"
        aria-rowcount={rows.length}
        containerRef={setViewport}
        containerClassName="h-full overflow-auto"
        className="grid min-w-full"
        style={{
          minWidth: `${minimumWidth}rem`,
          height: virtualBodyHeight + 40,
          gridTemplateRows: "2.5rem 1fr",
        }}
      >
        <TableHeader
          role="rowgroup"
          className="sticky top-0 z-20 grid bg-card shadow-[inset_0_-1px_0_0_var(--border)] [&_tr]:border-b-0"
        >
          <TableRow
            role="row"
            className="flex w-full border-b-0 hover:bg-transparent"
          >
            <TableHead
              role="columnheader"
              className="flex h-10 shrink-0 items-center p-0"
              style={{ flex: `0 0 ${EXPANDER_REM}rem` }}
            >
              <span className="sr-only">Expand record</span>
            </TableHead>
            {columns.map((column) => {
              const layout = columnLayout(column);
              const state = sort?.key === column.key ? sort.direction : null;
              const numeric = column.type === "number";
              return (
                <TableHead
                  role="columnheader"
                  key={column.key}
                  aria-sort={
                    state === "asc"
                      ? "ascending"
                      : state === "desc"
                        ? "descending"
                        : "none"
                  }
                  className={cn(
                    "flex h-10 min-w-0 items-center px-3 py-0",
                    numeric
                      ? "justify-end text-right"
                      : "justify-start text-left"
                  )}
                  style={{
                    flex: `${layout.grow} 0 ${layout.basisRem}rem`,
                  }}
                >
                  <button
                    type="button"
                    onClick={() => onSortChange(column.key)}
                    className={cn(
                      "group/sort flex min-w-0 items-center gap-1 rounded-sm hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                      numeric && "flex-row-reverse",
                      state && "text-foreground"
                    )}
                  >
                    <span
                      className={cn(
                        "min-w-0",
                        numeric
                          ? "text-right leading-tight whitespace-normal"
                          : "truncate"
                      )}
                    >
                      {column.label}
                    </span>
                    <SortIcon state={state} />
                  </button>
                </TableHead>
              );
            })}
          </TableRow>
        </TableHeader>
        <TableBody
          role="rowgroup"
          className="relative grid"
          style={{ height: virtualBodyHeight }}
        >
          {virtualRows.map((virtualRow) => {
            const row = rows[virtualRow.index];
            if (!row) return null;
            const key = rowKeys[virtualRow.index]!;
            const isOpen = expanded.has(key);
            const links = evidenceRecordLinks(metricKey ?? "", row.values);
            return (
              <TableRow
                role="row"
                key={key}
                data-index={virtualRow.index}
                ref={virtualizer.measureElement}
                aria-rowindex={virtualRow.index + 2}
                aria-expanded={isOpen}
                onClick={() => toggleRow(key)}
                className="absolute top-0 left-0 flex w-full cursor-pointer flex-wrap hover:bg-muted/20"
                style={{
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                <TableCell
                  role="cell"
                  className="flex h-11 shrink-0 items-center justify-center p-0"
                  style={{ flex: `0 0 ${EXPANDER_REM}rem` }}
                >
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    aria-expanded={isOpen}
                    aria-label={
                      isOpen ? "Hide full record" : "Show full record"
                    }
                    className="text-muted-foreground"
                    onClick={(event) => {
                      event.stopPropagation();
                      toggleRow(key);
                    }}
                  >
                    {isOpen ? <ChevronDown /> : <ChevronRight />}
                  </Button>
                </TableCell>
                {columns.map((column) => {
                  const layout = columnLayout(column);
                  const value = row.values[column.key];
                  const text = cellText(value, column.type);
                  const line = summaryLine(text);
                  return (
                    <TableCell
                      role="cell"
                      key={column.key}
                      className={cn(
                        "h-11 min-w-0 truncate px-3 py-3 tabular-nums",
                        column.type === "number" && "text-right"
                      )}
                      style={{
                        flex: `${layout.grow} 0 ${layout.basisRem}rem`,
                      }}
                      title={line}
                    >
                      {column.key === "ref" && value != null ? (
                        <div className="flex min-w-0 items-center gap-1">
                          <span className="min-w-0 truncate">
                            <RecordLink href={links[column.key]}>
                              {line}
                            </RecordLink>
                          </span>
                          <CopyValueButton
                            value={String(value)}
                            title="Copy ref"
                            errorMessage="Unable to copy ref"
                          />
                        </div>
                      ) : (
                        <RecordLink href={links[column.key]}>{line}</RecordLink>
                      )}
                    </TableCell>
                  );
                })}
                {isOpen ? (
                  <TableCell
                    role="cell"
                    aria-colspan={columns.length + 1}
                    // INVARIANT: selecting text here must not reach the row's
                    // expand toggle.
                    onClick={(event) => event.stopPropagation()}
                    className="w-full basis-full cursor-auto border-t bg-muted/40 px-3 py-3"
                  >
                    {/* INVARIANT: a fixed cap, not a vh — the window can
                        exceed the table's own height. */}
                    <dl className="grid max-h-96 gap-x-6 gap-y-2 overflow-y-auto overscroll-contain sm:grid-cols-[10rem_1fr]">
                      {columns.map((column) => {
                        const text = cellText(
                          row.values[column.key],
                          column.type
                        );
                        return (
                          <Fragment key={column.key}>
                            <dt className="text-sm text-muted-foreground">
                              {column.label}
                            </dt>
                            <dd className="text-sm break-words whitespace-pre-wrap">
                              <RecordLink
                                href={
                                  SUMMARY_ONLY_LINKS.has(column.key)
                                    ? undefined
                                    : links[column.key]
                                }
                              >
                                {text}
                              </RecordLink>
                            </dd>
                          </Fragment>
                        );
                      })}
                    </dl>
                  </TableCell>
                ) : null}
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
      {isFetchingNextPage ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center bg-card/80 p-3">
          <Spinner />
        </div>
      ) : null}
      {nextPageError ? (
        <div
          role="alert"
          className="absolute inset-x-0 bottom-0 flex items-center justify-center gap-3 border-t bg-card p-3"
        >
          <span>Unable to load more rows</span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void fetchNextPage()}
          >
            Retry
          </Button>
        </div>
      ) : pageLimitReached ? (
        <div className="absolute inset-x-0 bottom-0 border-t bg-card p-3 text-center text-sm text-muted-foreground">
          Showing the first 5,000 rows. Refine the selection or export the full
          result.
        </div>
      ) : null}
    </div>
  );
}
