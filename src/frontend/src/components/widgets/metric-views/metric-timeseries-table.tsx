import { useCallback, useEffect, useRef, useState } from "react";
import {
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
} from "lucide-react";

import { Button } from "@/components/ui/button";

import { formatMetricNumber } from "@/lib/format";
import { cn } from "@/lib/utils";
import {
  Table,
  TableBody,
  TableCell,
  TableFooter,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import {
  resolveMetricTimeseriesTableColumns,
  type MetricTimeseriesTableColumn,
} from "@/components/widgets/metric-views/metric-timeseries-table-model";
import type { MetricTimeseriesTableConfig } from "@/lib/metrics/timeseries-table";

export interface MetricTimeseriesTableProps {
  model: MetricTimeseriesModel;
  config?: MetricTimeseriesTableConfig;
  /**
   * Whether the rows overrun the box — the card above offers to let the table
   * out to full height, and only when that would do anything.
   */
  onVerticalOverflow?: (overflows: boolean) => void;
  onEvidence?: (
    metricKey: string,
    columnKey: string | null,
    bucketStart: string | null
  ) => void;
}

type Side = "start" | "end" | "up" | "down";

const SIDES: readonly Side[] = ["start", "end", "up", "down"];
const NOTHING_MORE: Record<Side, boolean> = {
  start: false,
  end: false,
  up: false,
  down: false,
};

const NO_GUTTERS = { start: false, end: false };

/** Sub-pixel scroll offsets are noise, not a side with content on it. */
const EDGE_SLACK_PX = 1;

/** A page that leaves the edge row or column on screen to keep the reader's place. */
const NEARLY_A_FRAME = 0.8;

/**
 * Every control sits in a gutter beside the scrollport rather than over it, so
 * none of them can hide a cell.
 *
 * INVARIANT: the end gutter stacks three — rows up at the top, columns forward
 * in the middle, rows down at the bottom — which needs about 104px of height to
 * keep them apart. Every caller renders the table taller than that; one that
 * does not gets three chevrons in a heap.
 */
const SIDE_CHROME: Record<
  Side,
  { icon: typeof ChevronLeft; label: string; place: string }
> = {
  start: {
    icon: ChevronLeft,
    label: "Show earlier columns",
    place: "start-1 top-1/2 -translate-y-1/2 rtl:rotate-180",
  },
  end: {
    icon: ChevronRight,
    label: "Show later columns",
    place: "end-1 top-1/2 -translate-y-1/2 rtl:rotate-180",
  },
  up: {
    icon: ChevronUp,
    label: "Show earlier rows",
    place: "top-1 end-1",
  },
  down: {
    icon: ChevronDown,
    label: "Show later rows",
    place: "bottom-1 end-1",
  },
};

/**
 * The gutters, wide enough for a control plus the inset it is placed at.
 *
 * INVARIANT: `GUTTER_PX` is what these classes render as. The measure pass
 * subtracts it to decide whether the table overflows, so the two disagreeing
 * makes the controls appear against a width the layout never had.
 */
const GUTTER_START = "ps-12";
const GUTTER_END = "pe-12";
const GUTTER_PX = 48;

/**
 * The bucket column, sticky at the start of every row, and the offset the
 * grand-total row sticks at to clear it.
 *
 * INVARIANT: the two widths are the same number. Wide enough for a full
 * `YYYY-MM-DD` plus the cells' own padding, and no wider — this column and the
 * gutters come out of one scrollport, which on a phone has a single data
 * column's room to spare between them.
 */
const BUCKET_COL = "w-24 max-w-24 min-w-24";
const PAST_BUCKET_COL = "start-24";

const BUCKET_LABEL = {
  day: "Day",
  week: "Week",
  month: "Month",
} as const;

const TONE_CLASS = {
  default: undefined,
  muted: "text-muted-foreground",
  success: "text-success",
  destructive: "text-destructive",
} as const;

function MetricTableValue({
  column,
  valueFor,
  onMetricClick,
}: {
  column: MetricTimeseriesTableColumn;
  valueFor: (metricKey: string) => number | null | undefined;
  onMetricClick?: (metricKey: string) => void;
}) {
  const hasValue = column.parts.some(
    (part) => part.kind === "metric" && valueFor(part.metricKey) != null
  );
  if (!hasValue) return <>—</>;

  return (
    <span>
      {column.parts.map((part, index) => {
        if (part.kind === "text") return <span key={index}>{part.text}</span>;
        const value = valueFor(part.metricKey);
        const metric = part.metric;
        if (value == null || !metric) {
          return (
            <span
              key={`${part.metricKey}-${index}`}
              className={TONE_CLASS.muted}
            >
              —
            </span>
          );
        }
        const content = (
          <>
            {part.prefix}
            {formatMetricNumber(value, metric.format)}
          </>
        );
        return onMetricClick ? (
          <button
            key={`${part.metricKey}-${index}`}
            type="button"
            className={cn(TONE_CLASS[part.tone], "hover:underline")}
            onClick={() => onMetricClick(part.metricKey)}
          >
            {content}
          </button>
        ) : (
          <span
            key={`${part.metricKey}-${index}`}
            className={TONE_CLASS[part.tone]}
          >
            {content}
          </span>
        );
      })}
    </span>
  );
}

export function MetricTimeseriesTable({
  model,
  config,
  onVerticalOverflow,
  onEvidence,
}: MetricTimeseriesTableProps) {
  const tableColumns = resolveMetricTimeseriesTableColumns(model, config);
  const grandTotals = new Map(
    model.metrics.map((metric, index) => [
      metric.metric_key,
      model.grandTotals[index],
    ])
  );
  const hasGrandTotal = tableColumns.some((column) =>
    column.parts.some(
      (part) =>
        part.kind === "metric" && grandTotals.get(part.metricKey) != null
    )
  );

  const wrapRef = useRef<HTMLDivElement | null>(null);
  const boxRef = useRef<HTMLDivElement | null>(null);
  const [more, setMore] = useState(NOTHING_MORE);
  const [gutters, setGutters] = useState(NO_GUTTERS);
  const measure = useCallback(() => {
    const box = boxRef.current;
    const wrap = wrapRef.current;
    if (!box || !wrap) return;
    // `scrollLeft` is negative in RTL.
    const across = Math.abs(box.scrollLeft);
    const acrossRoom = box.scrollWidth - box.clientWidth;
    const downRoom = box.scrollHeight - box.clientHeight;
    const next = {
      start: across > EDGE_SLACK_PX,
      end: across < acrossRoom - EDGE_SLACK_PX,
      up: box.scrollTop > EDGE_SLACK_PX,
      down: box.scrollTop < downRoom - EDGE_SLACK_PX,
    };
    // Keeping the object identity where the answer is unchanged; a new one per
    // scroll event re-renders on every frame of a drag.
    setMore((prev) =>
      SIDES.every((side) => prev[side] === next[side]) ? prev : next
    );

    // The wrapper's own width, not the scrollport's: the gutters are its
    // padding, so this figure is the one thing they cannot move. Deciding from
    // the scrollport feeds a gutter its own effect — the width it took makes
    // the overflow that keeps it — and the table then scrolls sideways forever
    // over a card that has long since grown wide enough.
    const rows = box.querySelector("table");
    const naturalWidth = rows?.offsetWidth ?? box.scrollWidth;
    const scrollsDown = downRoom > EDGE_SLACK_PX;
    // The table never wraps a cell (`min-w-max`), so its height owes nothing to
    // any of this and the end gutter can be subtracted without circling back.
    const roomAcross = wrap.clientWidth - (scrollsDown ? GUTTER_PX : 0);
    const scrollsAcross = naturalWidth > roomAcross + EDGE_SLACK_PX;
    setGutters((prev) =>
      prev.start === scrollsAcross && prev.end === (scrollsAcross || scrollsDown)
        ? prev
        : { start: scrollsAcross, end: scrollsAcross || scrollsDown }
    );

    onVerticalOverflow?.(scrollsDown);
  }, [onVerticalOverflow]);
  useEffect(() => {
    measure();
    const box = boxRef.current;
    if (!box) return;
    // `onScroll` would land on the table; the container is what scrolls, and
    // it takes no props of its own.
    box.addEventListener("scroll", measure, { passive: true });
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(measure);
    // Both sides of the comparison move: the box with the window, the table
    // with its data.
    observer?.observe(box);
    const inner = box.querySelector("table");
    if (inner) observer?.observe(inner);
    return () => {
      box.removeEventListener("scroll", measure);
      observer?.disconnect();
    };
    // `model` is rebuilt every render, so it must stay out of the deps.
  }, [measure]);

  function page(side: Side): void {
    const box = boxRef.current;
    if (!box) return;
    const forward = side === "end" || side === "down";
    if (side === "up" || side === "down") {
      const step = box.clientHeight * NEARLY_A_FRAME;
      box.scrollBy({ top: forward ? step : -step, behavior: "smooth" });
      return;
    }
    const step = box.clientWidth * NEARLY_A_FRAME;
    const rtl = getComputedStyle(box).direction === "rtl";
    const delta = forward ? step : -step;
    box.scrollBy({ left: rtl ? -delta : delta, behavior: "smooth" });
  }

  return (
    <div
      ref={wrapRef}
      className={cn(
        "relative h-full",
        gutters.start && GUTTER_START,
        gutters.end && GUTTER_END
      )}
    >
      {/* Rendered, not hidden: opacity would leave them in the tab order. */}
      {SIDES.filter((side) => more[side]).map((side) => {
        const { icon: Icon, label, place } = SIDE_CHROME[side];
        return (
          <Button
            key={side}
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label={label}
            title={label}
            onClick={() => page(side)}
            className={cn(
              "absolute z-40 rounded-full bg-card shadow-md",
              place
            )}
          >
            <Icon className="size-4" />
          </Button>
        );
      })}
      <Table
        className="min-w-max text-xs"
        containerClassName="h-full overflow-auto"
        containerRef={boxRef}
      >
      <TableHeader className="[&_tr]:border-b-0">
        {model.dimensions.length === 0 ? (
          <TableRow>
            <TableHead className={cn(BUCKET_COL, "sticky top-0 left-0 z-30 h-10 bg-card py-0 shadow-[inset_0_-1px_0_0_var(--border)] after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}>
              {BUCKET_LABEL[model.bucket]}
            </TableHead>
            {tableColumns.map((column, columnIndex) => (
              <TableHead
                key={column.key}
                className={cn(
                  "sticky top-0 z-20 h-10 min-w-24 bg-card py-0 text-right shadow-[inset_0_-1px_0_0_var(--border)]",
                  columnIndex > 0 &&
                    "before:absolute before:inset-y-0 before:left-0 before:w-px before:bg-border"
                )}
              >
                {column.label}
              </TableHead>
            ))}
          </TableRow>
        ) : tableColumns.length === 1 ? (
          <TableRow>
            <TableHead className={cn(BUCKET_COL, "sticky top-0 left-0 z-30 h-10 bg-card py-0 shadow-[inset_0_-1px_0_0_var(--border)] after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}>
              {BUCKET_LABEL[model.bucket]}
            </TableHead>
            {model.columns.map((column, index) => (
              <TableHead
                key={column.key}
                className={cn(
                  "sticky top-0 z-20 h-10 min-w-24 bg-card py-0 text-center shadow-[inset_0_-1px_0_0_var(--border)]",
                  index > 0 &&
                    "before:absolute before:inset-y-0 before:left-0 before:w-px before:bg-border"
                )}
              >
                {column.label}
              </TableHead>
            ))}
          </TableRow>
        ) : (
          <>
            <TableRow>
              <TableHead className={cn(BUCKET_COL, "sticky top-0 left-0 z-30 h-10 bg-card py-0 after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}>
                {BUCKET_LABEL[model.bucket]}
              </TableHead>
              {model.columns.map((column, index) => (
                <TableHead
                  key={column.key}
                  colSpan={tableColumns.length}
                  className={cn(
                    "sticky top-0 z-20 h-10 bg-card py-0 text-center after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-border",
                    index > 0 &&
                      "before:absolute before:inset-y-0 before:left-0 before:w-px before:bg-border"
                  )}
                >
                  {column.label}
                </TableHead>
              ))}
            </TableRow>
            <TableRow>
              <TableHead
                aria-hidden
                className={cn(BUCKET_COL, "sticky top-10 left-0 z-30 h-9 bg-card py-0 shadow-[inset_0_-1px_0_0_var(--border)] after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}
              />
              {model.columns.flatMap((column, columnIndex) =>
                tableColumns.map((tableColumn, tableColumnIndex) => (
                  <TableHead
                    key={`${column.key}-${tableColumn.key}`}
                    className={cn(
                      "sticky top-10 z-20 h-9 min-w-24 bg-card py-0 text-right after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-border",
                      (columnIndex > 0 || tableColumnIndex > 0) &&
                        "before:absolute before:inset-y-0 before:left-0 before:w-px before:bg-border"
                    )}
                  >
                    {tableColumn.label}
                  </TableHead>
                ))
              )}
            </TableRow>
          </>
        )}
      </TableHeader>
      <TableBody>
        {model.buckets.map((bucketStart) => (
          <TableRow key={bucketStart}>
            <TableCell className={cn(BUCKET_COL, "sticky left-0 z-10 bg-card px-2 py-1 font-medium tabular-nums after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}>
              {bucketStart}
            </TableCell>
            {model.columns.flatMap((column, columnIndex) =>
              tableColumns.map((tableColumn, tableColumnIndex) => {
                return (
                  <TableCell
                    key={`${column.key}-${tableColumn.key}`}
                    className={cn(
                      "px-2 py-1 text-right tabular-nums",
                      (columnIndex > 0 || tableColumnIndex > 0) && "border-l"
                    )}
                  >
                    <MetricTableValue
                      column={tableColumn}
                      valueFor={(metricKey) =>
                        column.points.get(metricKey)?.get(bucketStart)
                      }
                      onMetricClick={
                        column.remainder || !onEvidence
                          ? undefined
                          : (metricKey) =>
                              onEvidence(metricKey, column.key, bucketStart)
                      }
                    />
                  </TableCell>
                );
              })
            )}
          </TableRow>
        ))}
      </TableBody>
      {/* The top edge is what says the rows continue underneath; without it
          the pinned block reads as the end of the table. */}
      {/* Opaque, overriding the default half-transparent tint: the value cells
          carry no background of their own, so rows travelled under the totals
          and the row the footer covers showed through sliced along its middle.
          A frosted variant keeps that sliver visible by design — the totals
          need the row behind them gone, not softened. */}
      <TableFooter className="sticky bottom-0 z-20 bg-muted shadow-[inset_0_1px_0_0_var(--border)]">
        <TableRow>
          <TableCell className={cn(BUCKET_COL, "sticky left-0 z-10 bg-muted px-2 py-1 font-semibold after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}>
            Total
          </TableCell>
          {model.columns.flatMap((column, columnIndex) =>
            tableColumns.map((tableColumn, tableColumnIndex) => {
              return (
                <TableCell
                  key={`${column.key}-${tableColumn.key}`}
                  className={cn(
                    "px-2 py-1 text-right font-semibold tabular-nums",
                    (columnIndex > 0 || tableColumnIndex > 0) && "border-l"
                  )}
                >
                  <MetricTableValue
                    column={tableColumn}
                    valueFor={(metricKey) => column.totals.get(metricKey)}
                    onMetricClick={
                      column.remainder || !onEvidence
                        ? undefined
                        : (metricKey) => onEvidence(metricKey, column.key, null)
                    }
                  />
                </TableCell>
              );
            })
          )}
        </TableRow>
        {model.dimensions.length > 0 && hasGrandTotal ? (
          <TableRow>
            <TableCell className={cn(BUCKET_COL, "sticky left-0 z-10 bg-muted px-2 pt-1 pb-5 font-semibold after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border")}>
              Grand total
            </TableCell>
            <TableCell
              colSpan={model.columns.length * tableColumns.length}
              className="bg-muted px-2 pt-1 pb-5 text-left font-semibold tabular-nums"
            >
              {/* Stuck past the label column, since the cell spans every group
                  and its own start edge scrolls away. */}
              <span className={cn(PAST_BUCKET_COL, "sticky inline-flex flex-wrap items-center gap-1.5")}>
                {tableColumns.map((tableColumn, index) => (
                  <span
                    key={tableColumn.key}
                    className="inline-flex items-center gap-1.5"
                  >
                    {index > 0 ? (
                      <span className="text-muted-foreground">·</span>
                    ) : null}
                    <span>
                      {tableColumn.label}:{" "}
                      <MetricTableValue
                        column={tableColumn}
                        valueFor={(metricKey) => grandTotals.get(metricKey)}
                      />
                    </span>
                  </span>
                ))}
              </span>
            </TableCell>
          </TableRow>
        ) : null}
        </TableFooter>
      </Table>
    </div>
  );
}
