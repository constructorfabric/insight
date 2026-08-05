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
  onEvidence?: (
    metricKey: string,
    columnKey: string,
    bucketStart: string | null
  ) => void;
}

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

  return (
    <Table
      className="min-w-max text-xs"
      containerClassName="h-full overflow-auto"
    >
      <TableHeader className="[&_tr]:border-b-0">
        {model.dimensions.length === 0 ? (
          <TableRow>
            <TableHead className="sticky top-0 left-0 z-30 h-10 w-28 max-w-28 min-w-28 bg-card py-0 shadow-[inset_0_-1px_0_0_var(--border)] after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border">
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
            <TableHead className="sticky top-0 left-0 z-30 h-10 w-28 max-w-28 min-w-28 bg-card py-0 shadow-[inset_0_-1px_0_0_var(--border)] after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border">
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
              <TableHead className="sticky top-0 left-0 z-30 h-10 w-28 max-w-28 min-w-28 bg-card py-0 after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border">
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
                className="sticky top-10 left-0 z-30 h-9 w-28 max-w-28 min-w-28 bg-card py-0 shadow-[inset_0_-1px_0_0_var(--border)] after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border"
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
            <TableCell className="sticky left-0 z-10 w-28 max-w-28 min-w-28 bg-card px-2 py-1 font-medium tabular-nums after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border">
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
      <TableFooter>
        <TableRow>
          <TableCell className="sticky left-0 z-10 w-28 max-w-28 min-w-28 bg-muted px-2 py-1 font-semibold after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border">
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
            <TableCell className="sticky left-0 z-10 w-28 max-w-28 min-w-28 bg-muted px-2 pt-1 pb-5 font-semibold after:absolute after:inset-y-0 after:right-0 after:w-px after:bg-border">
              Grand total
            </TableCell>
            <TableCell
              colSpan={model.columns.length * tableColumns.length}
              className="bg-muted px-2 pt-1 pb-5 text-left font-semibold tabular-nums"
            >
              <span className="inline-flex flex-wrap items-center gap-1.5">
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
  );
}
