import { downloadBlob } from "@/lib/download";

export type MatrixCell = string | number | null | undefined;

export interface Matrix {
  columns: string[];
  rows: MatrixCell[][];
}

/**
 * Escape one cell for CSV.
 *
 * The leading-symbol guard is the load-bearing part: a spreadsheet treats a
 * cell opening with `=`, `+`, `-` or `@` as a formula, so a value that looks
 * like one is prefixed with an apostrophe. Shared so there is a single
 * escaping rule across every export this app writes.
 */
export function csvCell(value: MatrixCell): string {
  if (value == null) return "";
  const raw =
    typeof value === "number"
      ? Number.isFinite(value)
        ? String(value)
        : ""
      : /^[\t\r ]*[=+\-@]/.test(value)
        ? `'${value}`
        : value;
  return /[",\r\n]/.test(raw) ? `"${raw.replaceAll('"', '""')}"` : raw;
}

export function matrixToCsv(matrix: Matrix): string {
  return [matrix.columns, ...matrix.rows]
    .map((row) => row.map(csvCell).join(","))
    .join("\r\n");
}

export function downloadMatrixCsv(filename: string, matrix: Matrix): void {
  // The BOM is what makes Excel read it as UTF-8 rather than the local
  // codepage, which otherwise mangles every non-ASCII name in the file.
  const blob = new Blob(["﻿", matrixToCsv(matrix), "\r\n"], {
    type: "text/csv;charset=utf-8",
  });
  downloadBlob(blob, filename);
}

export async function downloadMatrixXlsx(
  filename: string,
  sheetName: string,
  matrix: Matrix,
): Promise<void> {
  const { Workbook } = await import("exceljs");
  const workbook = new Workbook();
  workbook.creator = "Insight";
  const sheet = workbook.addWorksheet(sheetName.slice(0, 31) || "Report");

  sheet.addRow(matrix.columns);
  for (const row of matrix.rows) {
    // A non-finite number reaches the sheet as a raw value and can make the
    // workbook unreadable, so it is dropped — the same answer the CSV writer
    // already gives it.
    sheet.addRow(
      row.map((cell) =>
        typeof cell === "number" && !Number.isFinite(cell) ? null : cell ?? null,
      ),
    );
  }

  const header = sheet.getRow(1);
  header.font = { bold: true };
  sheet.views = [{ state: "frozen", ySplit: 1 }];
  sheet.autoFilter = {
    from: { row: 1, column: 1 },
    to: { row: 1, column: Math.max(1, matrix.columns.length) },
  };
  matrix.columns.forEach((column, index) => {
    const widest = matrix.rows.reduce(
      (max, row) => Math.max(max, String(row[index] ?? "").length),
      column.length,
    );
    sheet.getColumn(index + 1).width = Math.min(40, Math.max(10, widest + 2));
  });

  const buffer = await workbook.xlsx.writeBuffer();
  downloadBlob(
    new Blob([buffer], {
      type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    }),
    filename,
  );
}
