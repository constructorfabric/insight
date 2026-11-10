import { useState } from "react";
import { Download, FileSpreadsheet, FileText } from "lucide-react";

import type { DateRange } from "@/api/period-to-date-range";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@gears-frontx/ui-kit";
import {
  downloadMatrixCsv,
  downloadMatrixXlsx,
  type Matrix,
} from "@/lib/export/matrix";
import {
  metricsExportFilename,
  type MetricsExportPage,
} from "@/lib/portal/metrics-view-export";
import { recordUsageEvent } from "@/telemetry";

const SHEET_NAME: Record<MetricsExportPage, string> = {
  metrics: "Metrics",
  "metrics-compare": "Compare",
};

export function MetricsExportMenu({
  page,
  period,
  matrix,
  disabled,
}: {
  page: MetricsExportPage;
  period: DateRange;
  matrix: Matrix;
  disabled: boolean;
}) {
  const [exporting, setExporting] = useState(false);

  function filename(extension: "csv" | "xlsx"): string {
    return metricsExportFilename(page, period, extension);
  }

  async function exportXlsx(): Promise<void> {
    recordUsageEvent("export", `${page}:xlsx`);
    setExporting(true);
    try {
      await downloadMatrixXlsx(filename("xlsx"), SHEET_NAME[page], matrix);
    } finally {
      setExporting(false);
    }
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled || exporting}
        render={
          <Button
            type="button"
            variant="outline"
            size="sm"
            aria-label="Download"
            icon={<Download />}
            loading={exporting}
          >
            Download
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => void exportXlsx()}>
          <FileSpreadsheet className="size-4" />
          Excel (.xlsx)
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => {
            recordUsageEvent("export", `${page}:csv`);
            downloadMatrixCsv(filename("csv"), matrix);
          }}
        >
          <FileText className="size-4" />
          CSV (.csv)
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
