import { FileSpreadsheet, FileText } from "lucide-react";

import type { ReportColumn, ReportPreviewResponse } from "@/api/reports-client";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

const PREVIEW_ROWS = 20;

interface ReportPreviewDialogProps {
  response: ReportPreviewResponse | null;
  open: boolean;
  period: { from: string; to: string };
  granularity: string;
  running: boolean;
  onOpenChange: (open: boolean) => void;
  onExport: (format: "csv" | "xlsx") => void;
}

export function ReportPreviewDialog({
  response,
  open,
  period,
  granularity,
  running,
  onOpenChange,
  onExport,
}: ReportPreviewDialogProps) {
  return (
    <Dialog open={open && response != null} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] flex-col gap-3 sm:max-w-[min(96vw,1200px)]">
        <DialogHeader>
          <DialogTitle>
            {response?.total_rows ?? 0} rows · showing the first{" "}
            {Math.min(PREVIEW_ROWS, response?.rows.length ?? 0)}
          </DialogTitle>
          <p className="text-xs text-muted-foreground">
            {granularity} · {period.from} to {period.to}
          </p>
        </DialogHeader>
        {response ? (
          <>
            <Table className="text-xs" containerClassName="overflow-auto">
              <TableHeader>
                <TableRow>
                  {response.columns.map((column) => (
                    <TableHead key={column.key} className="whitespace-nowrap">
                      {column.label}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {response.rows.slice(0, PREVIEW_ROWS).map((row, rowIndex) => (
                  <TableRow key={rowIndex}>
                    {response.columns.map((column, columnIndex) => (
                      <TableCell
                        key={column.key}
                        className="whitespace-nowrap tabular-nums"
                      >
                        {formatPreviewCell(row[columnIndex] ?? null, column)}
                      </TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <div className="flex items-center justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={running}
                onClick={() => onExport("csv")}
              >
                <FileText className="size-4" /> CSV
              </Button>
              <Button
                type="button"
                size="sm"
                disabled={running}
                onClick={() => onExport("xlsx")}
              >
                <FileSpreadsheet className="size-4" /> Excel
              </Button>
            </div>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function formatPreviewCell(
  value: string | number | null,
  column: ReportColumn
): string {
  if (value == null) return "—";
  if (typeof value === "string") return value;
  if (column.format === "percent") return `${value}%`;
  if (column.format === "currency") {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: "USD",
      maximumFractionDigits: 15,
    }).format(value);
  }
  return column.unit ? `${value} ${column.unit}` : String(value);
}
