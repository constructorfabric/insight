import { useState } from "react";
import { Download, FileSpreadsheet, FileText } from "lucide-react";

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@gears-frontx/ui-kit";
import { downloadMatrixCsv, downloadMatrixXlsx } from "@/lib/export/matrix";
import {
  metricsViewFilename,
  metricsViewMatrix,
  type MetricsViewExport,
} from "@/lib/portal/metrics-view-export";
import { recordUsageEvent } from "@/telemetry";

const SHEET_NAME = "Metrics view";

export function MetricsDownloadMenu({
  view,
  disabled,
}: {
  view: MetricsViewExport;
  disabled: boolean;
}) {
  const [exporting, setExporting] = useState(false);

  async function downloadXlsx(): Promise<void> {
    recordUsageEvent("export", "metrics-view:xlsx");
    setExporting(true);
    try {
      await downloadMatrixXlsx(
        metricsViewFilename(view.range, "xlsx"),
        SHEET_NAME,
        metricsViewMatrix(view),
      );
    } finally {
      setExporting(false);
    }
  }

  function downloadCsv(): void {
    recordUsageEvent("export", "metrics-view:csv");
    downloadMatrixCsv(metricsViewFilename(view.range, "csv"), metricsViewMatrix(view));
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
            icon={<Download />}
            loading={exporting}
          >
            Download
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => void downloadXlsx()}>
          <FileSpreadsheet className="size-4" />
          Excel (.xlsx)
        </DropdownMenuItem>
        <DropdownMenuItem onClick={downloadCsv}>
          <FileText className="size-4" />
          CSV (.csv)
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
