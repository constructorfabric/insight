import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";

import type {
  MetricEvidenceColumn,
  MetricEvidenceRow,
} from "@/api/metric-drilldown-client";
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
import { formatMetricNumber } from "@/lib/format";
import { cn } from "@/lib/utils";

function cellText(value: unknown, type: MetricEvidenceColumn["type"]): string {
  if (value == null) return "—";
  if (type === "number" && typeof value === "number") {
    return formatMetricNumber(value, "decimal");
  }
  if (typeof value === "boolean") return value ? "Yes" : "No";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

function columnLayout(column: MetricEvidenceColumn) {
  if (column.key === "ref") return { basisRem: 9, grow: 0 };
  if (column.key === "title") return { basisRem: 24, grow: 2 };
  if (column.key === "repository") return { basisRem: 16, grow: 1.25 };
  if (column.key === "author") return { basisRem: 12, grow: 1 };
  if (column.key === "date") return { basisRem: 8, grow: 1 };
  return { basisRem: 9, grow: 1 };
}

function CopyValueButton({ value }: { value: string }) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (resetTimer.current != null) window.clearTimeout(resetTimer.current);
    },
    []
  );

  async function copyValue(): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (resetTimer.current != null) window.clearTimeout(resetTimer.current);
      resetTimer.current = window.setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
      toast.error("Unable to copy ref");
    }
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      className="shrink-0 text-muted-foreground hover:text-foreground"
      aria-label={copied ? "Copied" : `Copy ${value}`}
      title={copied ? "Copied" : "Copy ref"}
      onClick={() => void copyValue()}
    >
      {copied ? <Check /> : <Copy />}
    </Button>
  );
}

export function MetricEvidenceTable({
  rows,
  columns,
  fetchNextPage,
  hasNextPage,
  isFetchingNextPage,
  nextPageError,
  pageLimitReached,
}: {
  rows: MetricEvidenceRow[];
  columns: MetricEvidenceColumn[];
  fetchNextPage: () => Promise<unknown>;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  nextPageError: boolean;
  pageLimitReached: boolean;
}) {
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => viewport,
    estimateSize: () => 44,
    overscan: 8,
  });
  const virtualRows = virtualizer.getVirtualItems();
  const virtualBodyHeight = virtualizer.getTotalSize();
  const last = virtualRows.at(-1)?.index ?? 0;
  const minimumWidth = columns.reduce((total, column) => {
    return total + columnLayout(column).basisRem;
  }, 0);

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
            {columns.map((column) => {
              const layout = columnLayout(column);
              return (
                <TableHead
                  role="columnheader"
                  key={column.key}
                  className={cn(
                    "flex h-10 min-w-0 items-center px-3 py-0",
                    column.type === "number"
                      ? "justify-end text-right"
                      : "justify-start text-left"
                  )}
                  style={{
                    flex: `${layout.grow} 0 ${layout.basisRem}rem`,
                  }}
                >
                  {column.label}
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
            return (
              <TableRow
                role="row"
                key={virtualRow.index}
                aria-rowindex={virtualRow.index + 2}
                className="absolute top-0 left-0 flex h-11 w-full hover:bg-transparent"
                style={{
                  transform: `translateY(${virtualRow.start}px)`,
                }}
              >
                {columns.map((column) => {
                  const layout = columnLayout(column);
                  const value = row.values[column.key];
                  const text = cellText(value, column.type);
                  return (
                    <TableCell
                      role="cell"
                      key={column.key}
                      className={cn(
                        "min-w-0 truncate px-3 py-3 tabular-nums",
                        column.type === "number" && "text-right"
                      )}
                      style={{
                        flex: `${layout.grow} 0 ${layout.basisRem}rem`,
                      }}
                      title={text}
                    >
                      {column.key === "ref" && value != null ? (
                        <div className="flex min-w-0 items-center gap-1">
                          <span className="min-w-0 truncate">{text}</span>
                          <CopyValueButton value={String(value)} />
                        </div>
                      ) : (
                        text
                      )}
                    </TableCell>
                  );
                })}
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
