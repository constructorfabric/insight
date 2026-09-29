/** The virtualized table the platform-usage surfaces share. */
import { useState, type ReactNode } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, ChevronsUpDown } from "lucide-react";

import type { SortDirection, UsageOrder } from "@/api/usage-client";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { visitorLabel, type VisitorNaming } from "@/lib/portal/visitor-label";
import { cn } from "@/lib/utils";

const ROW_HEIGHT = 40;

export interface Column<T, K extends string = string> {
  header: string;
  width?: number;
  align?: "left" | "right";
  sortKey?: K;
  cell: (row: T, index: number) => ReactNode;
}

function SortIcon({
  direction,
  muted,
}: {
  direction: SortDirection | null;
  muted: boolean;
}) {
  const className = cn("size-3.5 shrink-0", muted && "text-muted-foreground");
  if (direction === "asc") return <ArrowUp className={className} />;
  if (direction === "desc") return <ArrowDown className={className} />;
  return (
    <ChevronsUpDown className="size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/sort:opacity-100" />
  );
}

const ARIA_SORT = { asc: "ascending", desc: "descending" } as const;

export function VirtualTable<T, K extends string = string>({
  rows,
  columns,
  rowKey,
  label,
  order,
  orderIsDefault = false,
  onSort,
  pending = false,
}: {
  rows: T[];
  columns: Column<T, K>[];
  rowKey: (row: T, index: number) => string;
  label: string;
  order?: UsageOrder<K>;
  orderIsDefault?: boolean;
  onSort?: (key: K) => void;
  pending?: boolean;
}) {
  // State, not a ref: the virtualizer re-reads the scroll element once it
  // exists, and a ref never re-renders to tell it.
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => viewport,
    estimateSize: () => ROW_HEIGHT,
    overscan: 8,
  });
  const bodyHeight = virtualizer.getTotalSize();

  const cellClass = (column: Column<T, K>) =>
    cn("truncate", column.align === "right" ? "text-right" : "");
  const cellStyle = (column: Column<T, K>) => ({
    flex: column.width ? `0 0 ${column.width}rem` : "1 1 0%",
  });

  return (
    <Table
      aria-label={label}
      aria-busy={pending || undefined}
      containerRef={setViewport}
      containerClassName="max-h-90 overflow-auto rounded-md border"
      className="grid min-w-full"
      style={{
        height: bodyHeight + ROW_HEIGHT,
        gridTemplateRows: `${ROW_HEIGHT}px 1fr`,
      }}
    >
      <TableHeader className="sticky top-0 z-10 grid bg-background">
        <TableRow className="flex w-full">
          {columns.map((column) => {
            const sortKey = column.sortKey;
            const direction = sortKey && order?.sort === sortKey ? order.direction : null;
            return (
              <TableHead
                key={column.header}
                aria-sort={sortKey ? (direction ? ARIA_SORT[direction] : "none") : undefined}
                data-order={direction ? (orderIsDefault ? "default" : "chosen") : undefined}
                className={cn(cellClass(column), "flex h-10 items-center")}
                style={cellStyle(column)}
              >
                {sortKey && onSort ? (
                  <button
                    type="button"
                    onClick={() => onSort(sortKey)}
                    className={cn(
                      "group/sort flex min-w-0 items-center gap-1 rounded-sm hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                      direction && !orderIsDefault && "text-foreground",
                    )}
                  >
                    <span className="truncate">{column.header}</span>
                    <SortIcon direction={direction} muted={orderIsDefault} />
                  </button>
                ) : (
                  column.header
                )}
              </TableHead>
            );
          })}
        </TableRow>
      </TableHeader>
      <TableBody
        className={cn("relative grid transition-opacity", pending && "opacity-60")}
        style={{ height: bodyHeight }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const row = rows[virtualRow.index];
          if (!row) return null;
          return (
            <TableRow
              key={rowKey(row, virtualRow.index)}
              data-index={virtualRow.index}
              ref={virtualizer.measureElement}
              className="absolute top-0 left-0 flex w-full"
              style={{ transform: `translateY(${virtualRow.start}px)` }}
            >
              {columns.map((column) => (
                <TableCell
                  key={column.header}
                  className={cn(cellClass(column), "flex items-center")}
                  style={cellStyle(column)}
                >
                  {column.cell(row, virtualRow.index)}
                </TableCell>
              ))}
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

/**
 * A row is one line high and its value often is not: the cell shows what fits
 * and the tooltip carries the whole of it.
 *
 * No `TooltipProvider` — the app root mounts one, and a second per cell would
 * be built twice per visible virtualized row.
 */
export function TruncatedCell({
  children,
  detail,
  detailClassName,
}: {
  children: ReactNode;
  detail: ReactNode;
  detailClassName?: string;
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={<span className="truncate" />}>
        {children}
      </TooltipTrigger>
      <TooltipContent
        className={cn(
          "max-h-[var(--available-height)] items-start overflow-y-auto overscroll-contain",
          detailClassName ?? "font-mono text-xs",
        )}
      >
        {detail}
      </TooltipContent>
    </Tooltip>
  );
}

/** Whoever the row is about, named the way every usage surface names them. */
export function PersonName({ row }: { row: VisitorNaming }) {
  const { label, detail } = visitorLabel(row);

  return <TruncatedCell detail={detail}>{label}</TruncatedCell>;
}
