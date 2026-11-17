import { useState } from "react";
import { Download, FileSpreadsheet, FileText } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  toast,
} from "@gears-frontx/ui-kit";
import { downloadMatrixCsv, downloadMatrixXlsx } from "@/lib/export/matrix";
import {
  metricsViewFilename,
  metricsViewMatrix,
  type MetricsViewExport,
} from "@/lib/portal/metrics-view-export";
import { recordUsageEvent } from "@/telemetry";

export function MetricsDownloadMenu({
  view,
  disabled,
}: {
  view: MetricsViewExport;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const [exporting, setExporting] = useState(false);

  async function downloadXlsx(): Promise<void> {
    recordUsageEvent("export", "metrics-view:xlsx");
    setExporting(true);
    try {
      await downloadMatrixXlsx(
        metricsViewFilename(view.range, "xlsx"),
        t("metrics_view.title"),
        metricsViewMatrix(view),
      );
    } catch {
      toast.add({ type: "error", title: t("metrics_view.download.failed") });
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
            {t("common.actions.download")}
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => void downloadXlsx()}>
          <FileSpreadsheet className="size-4" />
          {t("metrics_view.download.xlsx")}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={downloadCsv}>
          <FileText className="size-4" />
          {t("metrics_view.download.csv")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
