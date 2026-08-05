import { Workbook } from "exceljs";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { downloadMetricTimeseriesCsv } from "@/components/widgets/metric-views/metric-timeseries-csv";
import {
  downloadBlob,
  metricTimeseriesFilename,
} from "@/components/widgets/metric-views/metric-timeseries-export";
import {
  groupedTimeseriesModel,
  RANGE,
} from "@/components/widgets/metric-views/metric-timeseries.test-fixtures";
import { downloadMetricTimeseriesXlsx } from "@/components/widgets/metric-views/metric-timeseries-xlsx";

vi.mock(
  "@/components/widgets/metric-views/metric-timeseries-export",
  async (importOriginal) => {
    const actual =
      await importOriginal<
        typeof import("@/components/widgets/metric-views/metric-timeseries-export")
      >();
    return { ...actual, downloadBlob: vi.fn() };
  }
);

const mockedDownloadBlob = vi.mocked(downloadBlob);

function modelWithOther() {
  const model = groupedTimeseriesModel();
  return {
    ...model,
    columns: [
      ...model.columns,
      {
        key: "__remainder__",
        colorSeed: "timeseries:remainder",
        label: "Other",
        remainder: true,
        points: new Map([["git.commits", new Map([["2026-04-20", 1]])]]),
        totals: new Map([
          ["git.commits", 1],
          ["git.lines_added", null],
        ]),
      },
    ],
  };
}

describe("timeseries exports", () => {
  beforeEach(() => {
    mockedDownloadBlob.mockReset();
  });

  it("sanitizes filenames", () => {
    expect(metricTimeseriesFilename(" Git / output ", RANGE, "csv")).toBe(
      "Git-output_2026-04-20_2026-05-04.csv"
    );
    expect(metricTimeseriesFilename(" / ", RANGE, "xlsx")).toBe(
      "timeseries_2026-04-20_2026-05-04.xlsx"
    );
  });

  it("exports grouped CSV data without footer rows", async () => {
    downloadMetricTimeseriesCsv("output", modelWithOther(), RANGE);
    expect(mockedDownloadBlob).toHaveBeenCalledOnce();
    const [blob, filename] = mockedDownloadBlob.mock.calls[0] ?? [];
    expect(filename).toBe("output_2026-04-20_2026-05-04.csv");
    expect(await blob?.text()).toContain(
      "Week,org/repo-a — Commits,org/repo-a — Lines added"
    );
    expect(await blob?.text()).toContain("Other — Commits,Other — Lines added");
    expect(await blob?.text()).not.toContain("\r\nTotal,");
    expect(await blob?.text()).not.toContain("Grand total");
  });

  it("exports a formatted workbook with merged grouped headers", async () => {
    await downloadMetricTimeseriesXlsx("output", modelWithOther(), RANGE);
    expect(mockedDownloadBlob).toHaveBeenCalledOnce();
    const [blob, filename] = mockedDownloadBlob.mock.calls[0] ?? [];
    expect(filename).toBe("output_2026-04-20_2026-05-04.xlsx");
    const workbook = new Workbook();
    await workbook.xlsx.load(await blob!.arrayBuffer());
    const sheet = workbook.getWorksheet("Timeseries");
    expect(sheet?.getCell("A1").value).toBe("Week");
    expect(sheet?.getCell("B1").value).toBe("org/repo-a");
    expect(sheet?.getCell("B2").value).toBe("Commits");
    expect(sheet?.getCell("F1").value).toBe("Other");
    expect(sheet?.getCell("A6").value).toBe("Total");
    expect(sheet?.getCell("A7").value).toBe("Grand total");
    expect(sheet?.views[0]).toMatchObject({ xSplit: 1, ySplit: 2 });
  });
});
