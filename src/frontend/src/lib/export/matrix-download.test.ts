import { Workbook } from "exceljs";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/download", () => ({ downloadBlob: vi.fn() }));

import { downloadBlob } from "@/lib/download";
import { downloadMatrixCsv, downloadMatrixXlsx } from "@/lib/export/matrix";

const mocked = vi.mocked(downloadBlob);

const matrix = {
  columns: ["Person", "Period", "Commits"],
  rows: [
    ["Jane Doe", "2026-Q1", 4],
    ["=Sam", "2026-Q1", null],
  ],
};

beforeEach(() => mocked.mockReset());

describe("downloadMatrixCsv", () => {
  it("writes the file under the name it was given", async () => {
    downloadMatrixCsv("report.csv", matrix);
    expect(mocked).toHaveBeenCalledTimes(1);
    expect(mocked.mock.calls[0]?.[1]).toBe("report.csv");
  });

  it("leads with a BOM, so a spreadsheet reads it as UTF-8", async () => {
    downloadMatrixCsv("report.csv", matrix);
    const blob = mocked.mock.calls[0]?.[0] as Blob;
    // Checked as bytes: a text decoder strips the mark, which is the whole
    // reason it is there — Excel reads the bytes, not a decoded string.
    const head = new Uint8Array((await blob.arrayBuffer()).slice(0, 3));
    expect([...head]).toEqual([0xef, 0xbb, 0xbf]);

    const text = await blob.text();
    expect(text).toContain("Person,Period,Commits");
    // The escaping rule travels with the writer.
    expect(text).toContain("'=Sam");
  });
});

describe("downloadMatrixXlsx", () => {
  it("writes a sheet with a header row and every value row", async () => {
    await downloadMatrixXlsx("report.xlsx", "Report", matrix);
    expect(mocked.mock.calls[0]?.[1]).toBe("report.xlsx");

    const workbook = new Workbook();
    const blob = mocked.mock.calls[0]?.[0] as Blob;
    await workbook.xlsx.load(await blob.arrayBuffer());
    const sheet = workbook.getWorksheet("Report");
    expect(sheet).toBeDefined();
    expect(sheet?.getRow(1).values).toEqual([
      undefined,
      "Person",
      "Period",
      "Commits",
    ]);
    expect(sheet?.getRow(2).getCell(3).value).toBe(4);
    expect(sheet?.getRow(1).font?.bold).toBe(true);
  });

  it("drops a non-finite number rather than writing it into a cell", async () => {
    // ExcelJS writes a number straight through, and Infinity in a cell can
    // make the workbook unreadable. The CSV writer already refuses it.
    await downloadMatrixXlsx("report.xlsx", "Report", {
      columns: ["Person", "Ratio"],
      rows: [["Jane Doe", Number.POSITIVE_INFINITY], ["Sam Smith", Number.NaN]],
    });
    const workbook = new Workbook();
    await workbook.xlsx.load(
      await (mocked.mock.calls[0]?.[0] as Blob).arrayBuffer(),
    );
    const sheet = workbook.getWorksheet("Report");
    expect(sheet?.getRow(2).getCell(2).value).toBeNull();
    expect(sheet?.getRow(3).getCell(2).value).toBeNull();
  });

  it("keeps the sheet name inside the length a workbook allows", async () => {
    await downloadMatrixXlsx("r.xlsx", "x".repeat(60), matrix);
    const workbook = new Workbook();
    await workbook.xlsx.load(
      await (mocked.mock.calls[0]?.[0] as Blob).arrayBuffer(),
    );
    expect(workbook.worksheets[0]?.name.length).toBeLessThanOrEqual(31);
  });
});
