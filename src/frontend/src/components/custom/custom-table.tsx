import { useState } from "react";
import { ArrowDown, ArrowUp } from "lucide-react";

import type { MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";
import { nextOrder, ordered, type RowOrder } from "@/lib/custom/row-order";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export interface CustomTableProps {
  result: MetricResult;
}

/** Beyond this the card is a scroll bar, and the DOM pays for every row. */
const SHOWN = 200;

/**
 * One cell.
 *
 * A value that is a URL is the row's way out - to the pull request, the issue,
 * the page it counted - so it is a link rather than text to copy by hand.
 * Nothing is composed here: only a value that already is an absolute http(s)
 * URL becomes one.
 */
function Cell({ value, unit }: { value: unknown; unit: string }) {
  if (value === null || value === undefined) return <>{"\u2014"}</>;
  if (unit) return <>{groupedNumber(value, unit)}</>;

  const text = String(value);
  if (!/^https?:\/\/\S+$/.test(text)) return <>{text}</>;

  return (
    <a
      href={text}
      target="_blank"
      rel="noreferrer noopener"
      className="underline decoration-dotted underline-offset-4"
      onClick={(event) => event.stopPropagation()}
    >
      {text}
    </a>
  );
}

/** A column header that orders the table: up, down, and back as it came. */
function Sortable({
  column,
  index,
  order,
  onOrder,
}: {
  column: string;
  index: number;
  order: RowOrder | undefined;
  onOrder: (order: RowOrder | undefined) => void;
}) {
  const chosen = order?.column === index ? order.direction : "none";

  return (
    <TableHead aria-sort={chosen}>
      <button
        type="button"
        className="inline-flex items-center gap-1 hover:underline"
        aria-label={
          chosen === "none"
            ? `Order by ${column}`
            : `Order by ${column}, now ${chosen}`
        }
        // SAFETY: the card around a widget opens the drilldown on any click
        // or Enter inside it. Ordering the rows is not that decision.
        onClick={(event) => {
          event.stopPropagation();
          onOrder(nextOrder(order, index));
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") event.stopPropagation();
        }}
      >
        {column}
        {/* The slot is there whether or not an arrow is in it: a column that
            widened when it was ordered by would shift every column beside it. */}
        <span className="inline-flex size-3 shrink-0 items-center justify-center">
          {chosen === "ascending" ? <ArrowUp className="size-3" aria-hidden /> : null}
          {chosen === "descending" ? (
            <ArrowDown className="size-3" aria-hidden />
          ) : null}
        </span>
      </button>
    </TableHead>
  );
}

export function CustomTable({ result }: CustomTableProps) {
  const [order, setOrder] = useState<RowOrder | undefined>(undefined);
  const rows = ordered(result.rows, order).slice(0, SHOWN);

  return (
    <>
      {result.rows.length > SHOWN ? (
        <p className={cn(TEXT_LABEL, "mb-2")}>
          First {groupedNumber(SHOWN)} of {groupedNumber(result.rows.length)}{" "}
          rows
        </p>
      ) : null}
      {/* The wrapper stays out of the scrolling: whatever holds the table
          scrolls it, and the header sticks to that, so the columns can be
          re-ordered from anywhere in a long result. */}
      <Table containerClassName="overflow-visible">
        <TableHeader className="sticky top-0 z-10 bg-card shadow-[inset_0_-1px_0_0_var(--border)] [&_tr]:border-b-0">
          <TableRow>
            <TableHead className="w-0 text-muted-foreground">#</TableHead>
            {result.columns.map((column, index) => (
              <Sortable
                key={column}
                column={column}
                index={index}
                order={order}
                onOrder={setOrder}
              />
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row, rowIndex) => (
            <TableRow key={rowIndex}>
              <TableCell className="w-0 pe-3 text-end tabular-nums text-muted-foreground">
                {rowIndex + 1}
              </TableCell>
              {result.columns.map((column, columnIndex) => (
                <TableCell key={columnIndex}>
                  {columnIndex < row.length ? (
                    <Cell
                      value={row[columnIndex]}
                      unit={unitFor(result.percents, column)}
                    />
                  ) : (
                    ""
                  )}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </>
  );
}
