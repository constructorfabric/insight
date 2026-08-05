import { useState } from "react";
import {
  ChartColumn,
  Download,
  FileSpreadsheet,
  FileText,
  Table2,
} from "lucide-react";

import type { DateRange } from "@/api/period-to-date-range";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { ChartEmpty } from "@/components/widgets/metric-views/chart-empty";
import { MetricTimeseriesChart } from "@/components/widgets/metric-views/metric-timeseries-chart";
import { downloadMetricTimeseriesCsv } from "@/components/widgets/metric-views/metric-timeseries-csv";
import type { MetricTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries-model";
import type { TimeseriesPresentation } from "@/components/widgets/metric-views/metric-timeseries-presentation";
import { MetricTimeseriesTable } from "@/components/widgets/metric-views/metric-timeseries-table";
import { downloadMetricTimeseriesXlsx } from "@/components/widgets/metric-views/metric-timeseries-xlsx";
import type { MetricTimeseriesTableConfig } from "@/lib/metrics/timeseries-table";
import type { MetricTimeseriesChartConfig } from "@/lib/metrics/timeseries-chart";

export function TimeseriesPresentationToggle({
  presentation,
  onChange,
}: {
  presentation: TimeseriesPresentation;
  onChange: (presentation: TimeseriesPresentation) => void;
}) {
  return (
    <ToggleGroup
      value={[presentation]}
      onValueChange={(values) => {
        const next = Array.isArray(values) ? values[0] : values;
        if (next === "table" || next === "chart") onChange(next);
      }}
      variant="outline"
      size="sm"
    >
      <ToggleGroupItem value="chart" aria-label="Chart view" title="Chart view">
        <ChartColumn className="size-4" />
        Chart
      </ToggleGroupItem>
      <ToggleGroupItem value="table" aria-label="Table view" title="Table view">
        <Table2 className="size-4" />
        Table
      </ToggleGroupItem>
    </ToggleGroup>
  );
}

export function TimeseriesExportMenu({
  id,
  model,
  range,
  disabled,
}: {
  id: string;
  model: MetricTimeseriesModel;
  range: DateRange;
  disabled: boolean;
}) {
  const [isExporting, setIsExporting] = useState(false);

  async function exportXlsx(): Promise<void> {
    setIsExporting(true);
    try {
      await downloadMetricTimeseriesXlsx(id, model, range);
    } finally {
      setIsExporting(false);
    }
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        disabled={disabled || isExporting}
        render={
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="Export"
            title="Export"
          >
            {isExporting ? (
              <Spinner className="size-4" />
            ) : (
              <Download className="size-4" />
            )}
          </Button>
        }
      />
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={() => void exportXlsx()}>
          <FileSpreadsheet className="size-4" />
          Excel (.xlsx)
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => downloadMetricTimeseriesCsv(id, model, range)}
        >
          <FileText className="size-4" />
          CSV (.csv)
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function TimeseriesBody({
  isPending,
  isFetching,
  isError,
  onRetry,
  empty,
  presentation,
  model,
  selectedMetricKey,
  multiMetric,
  table,
  onEvidence,
}: {
  isPending: boolean;
  isFetching: boolean;
  isError: boolean;
  onRetry: () => void;
  empty: boolean;
  presentation: TimeseriesPresentation;
  model: MetricTimeseriesModel;
  selectedMetricKey: string;
  multiMetric: MetricTimeseriesChartConfig["multiMetric"];
  table?: MetricTimeseriesTableConfig;
  onEvidence?: (
    metricKey: string,
    columnKey: string,
    bucketStart: string | null
  ) => void;
}) {
  return (
    <div className="relative min-h-0 flex-1">
      {isFetching && !isPending ? (
        <div className="pointer-events-none absolute inset-0 z-40 flex items-center justify-center">
          <Spinner className="size-8 text-muted-foreground" />
        </div>
      ) : null}
      {isPending ? (
        <div className="flex h-full items-center justify-center">
          <Spinner className="size-10 text-muted-foreground" />
        </div>
      ) : isError ? (
        <div className="flex h-full items-center justify-center">
          <ComingSoon
            state="error"
            label="Unable to load timeseries"
            onRetry={onRetry}
          />
        </div>
      ) : empty ? (
        <ChartEmpty message="No data in this period" className="h-full" />
      ) : presentation === "table" ? (
        <MetricTimeseriesTable
          model={model}
          config={table}
          onEvidence={onEvidence}
        />
      ) : (
        <MetricTimeseriesChart
          model={model}
          selectedMetricKey={selectedMetricKey}
          multiMetric={multiMetric}
          onEvidence={onEvidence}
        />
      )}
    </div>
  );
}
