import type { Cell, FillPattern, Row, Worksheet } from "exceljs";

import type { MetricFormat } from "@/api/metric-results-client";
import type { DateRange } from "@/api/period-to-date-range";
import {
  downloadBlob,
  metricTimeseriesFilename,
} from "@/components/widgets/metric-views/metric-timeseries-export";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import { formatMetricNumber } from "@/lib/format";

const BUCKET_HEADER = {
  day: "Day",
  week: "Week",
  month: "Month",
} as const;

const HEADER_FILL: FillPattern = {
  type: "pattern",
  pattern: "solid",
  fgColor: { argb: "FFF3F4F6" },
};

const SUBHEADER_FILL: FillPattern = {
  type: "pattern",
  pattern: "solid",
  fgColor: { argb: "FFF9FAFB" },
};

const TOTAL_FILL: FillPattern = {
  type: "pattern",
  pattern: "solid",
  fgColor: { argb: "FFF3F4F6" },
};

const BORDER = {
  top: { style: "thin" as const, color: { argb: "FFD1D5DB" } },
  left: { style: "thin" as const, color: { argb: "FFD1D5DB" } },
  bottom: { style: "thin" as const, color: { argb: "FFD1D5DB" } },
  right: { style: "thin" as const, color: { argb: "FFD1D5DB" } },
};

function numberFormat(format: MetricFormat): string {
  if (format === "currency") return "$#,##0";
  if (format === "percent") return '#,##0"%"';
  if (format === "decimal") return "#,##0.0";
  return "#,##0";
}

function styleCell(cell: Cell): void {
  cell.border = BORDER;
  cell.alignment = { vertical: "middle" };
}

function styleHeaderRow(
  row: Row,
  columnCount: number,
  fill: FillPattern
): void {
  row.height = 24;
  for (let index = 1; index <= columnCount; index += 1) {
    const cell = row.getCell(index);
    styleCell(cell);
    cell.fill = fill;
    cell.font = { bold: true };
    cell.alignment = {
      horizontal: index === 1 ? "left" : "center",
      vertical: "middle",
      wrapText: true,
    };
  }
}

function styleDataRow(
  row: Row,
  model: MetricTimeseriesModel,
  bold = false
): void {
  row.getCell(1).font = { bold: true };
  const columnCount = 1 + model.columns.length * model.metrics.length;
  for (let index = 1; index <= columnCount; index += 1) {
    const cell = row.getCell(index);
    styleCell(cell);
    if (bold) {
      cell.font = { bold: true };
      cell.fill = TOTAL_FILL;
    }
    if (index > 1) {
      const metric = model.metrics[(index - 2) % model.metrics.length];
      cell.alignment = { horizontal: "right", vertical: "middle" };
      if (metric) cell.numFmt = numberFormat(metric.format);
    }
  }
}

function addHeaders(
  worksheet: Worksheet,
  model: MetricTimeseriesModel
): number {
  const grouped = model.dimensions.length > 0;
  const multiMetric = model.metrics.length > 1;
  const columnCount = 1 + model.columns.length * model.metrics.length;

  if (grouped && multiMetric) {
    const groupHeader = worksheet.addRow([
      BUCKET_HEADER[model.bucket],
      ...model.columns.flatMap((column) => [
        column.label,
        ...Array(model.metrics.length - 1).fill(null),
      ]),
    ]);
    const metricHeader = worksheet.addRow([
      null,
      ...model.columns.flatMap(() =>
        model.metrics.map((metric) => metric.label)
      ),
    ]);
    styleHeaderRow(groupHeader, columnCount, HEADER_FILL);
    styleHeaderRow(metricHeader, columnCount, SUBHEADER_FILL);
    worksheet.mergeCells(1, 1, 2, 1);
    model.columns.forEach((_, index) => {
      const start = 2 + index * model.metrics.length;
      worksheet.mergeCells(1, start, 1, start + model.metrics.length - 1);
    });
    return 2;
  }

  const header = worksheet.addRow([
    BUCKET_HEADER[model.bucket],
    ...model.columns.flatMap((column) =>
      model.metrics.map((metric) => (grouped ? column.label : metric.label))
    ),
  ]);
  styleHeaderRow(header, columnCount, HEADER_FILL);
  return 1;
}

function configureColumns(
  worksheet: Worksheet,
  model: MetricTimeseriesModel
): void {
  worksheet.getColumn(1).width = 14;
  model.columns.forEach((column, columnIndex) => {
    model.metrics.forEach((metric, metricIndex) => {
      const index = 2 + columnIndex * model.metrics.length + metricIndex;
      const labelLength =
        model.dimensions.length > 0 && model.metrics.length === 1
          ? column.label.length
          : metric.label.length;
      worksheet.getColumn(index).width = Math.min(
        36,
        Math.max(14, labelLength + 3)
      );
    });
  });
}

function addTimeseriesSheet(
  workbook: import("exceljs").Workbook,
  model: MetricTimeseriesModel
): void {
  const worksheet = workbook.addWorksheet("Timeseries");
  const headerRows = addHeaders(worksheet, model);
  configureColumns(worksheet, model);
  worksheet.views = [
    {
      state: "frozen",
      xSplit: 1,
      ySplit: headerRows,
      topLeftCell: `B${headerRows + 1}`,
    },
  ];

  for (const bucketStart of model.buckets) {
    const row = worksheet.addRow([
      bucketStart,
      ...model.columns.flatMap((column) =>
        model.metrics.map(
          (metric) =>
            column.points.get(metric.metric_key)?.get(bucketStart) ?? null
        )
      ),
    ]);
    styleDataRow(row, model);
  }

  const totalRow = worksheet.addRow([
    "Total",
    ...model.columns.flatMap((column) =>
      model.metrics.map(
        (metric) => column.totals.get(metric.metric_key) ?? null
      )
    ),
  ]);
  styleDataRow(totalRow, model, true);

  if (
    model.dimensions.length > 0 &&
    model.grandTotals.some((value) => value != null)
  ) {
    const columnCount = 1 + model.columns.length * model.metrics.length;
    const grandTotalText = model.metrics
      .map((metric, index) => {
        const value = model.grandTotals[index];
        return `${metric.label}: ${value == null ? "—" : formatMetricNumber(value, metric.format)}`;
      })
      .join(" · ");
    const grandTotalRow = worksheet.addRow(["Grand total", grandTotalText]);
    styleDataRow(grandTotalRow, model, true);
    if (columnCount > 2) {
      worksheet.mergeCells(
        grandTotalRow.number,
        2,
        grandTotalRow.number,
        columnCount
      );
    }
    grandTotalRow.getCell(2).alignment = {
      horizontal: "left",
      vertical: "middle",
    };
    grandTotalRow.getCell(2).numFmt = "General";
  }
}

export async function downloadMetricTimeseriesXlsx(
  id: string,
  model: MetricTimeseriesModel,
  range: DateRange
): Promise<void> {
  const { Workbook } = await import("exceljs");
  const workbook = new Workbook();
  workbook.creator = "Insight";
  workbook.created = new Date();
  addTimeseriesSheet(workbook, model);
  const buffer = await workbook.xlsx.writeBuffer();
  const blob = new Blob([buffer], {
    type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  });
  downloadBlob(blob, metricTimeseriesFilename(id, range, "xlsx"));
}
