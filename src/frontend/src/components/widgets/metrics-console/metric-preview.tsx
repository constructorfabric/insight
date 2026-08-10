import { useTranslation } from "react-i18next";

import type { MetricResultsResponse } from "@/api/metric-results-client";
import {
  Empty,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { TableIcon } from "lucide-react";

/** Flatten the `period` views of a results response into entity/value rows. */
function periodRows(
  response: MetricResultsResponse
): Array<{ entityId: string; value: number | null }> {
  const rows: Array<{ entityId: string; value: number | null }> = [];
  for (const metric of response.metrics) {
    for (const view of metric.views) {
      if (view.view === "period") {
        for (const entry of view.values) {
          rows.push({ entityId: entry.entity_id, value: entry.value });
        }
      }
    }
  }
  return rows;
}

export function MetricPreview({ result }: { result: MetricResultsResponse }) {
  const { t } = useTranslation();
  const rows = periodRows(result);

  if (rows.length === 0) {
    return (
      <Empty className="min-h-40">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <TableIcon />
          </EmptyMedia>
          <EmptyTitle>{t("metrics_console.preview.empty")}</EmptyTitle>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="font-mono text-xs">
              {t("metrics_console.preview.entity_column")}
            </TableHead>
            <TableHead className="font-mono text-xs">
              {t("metrics_console.preview.value_column")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row, index) => (
            <TableRow key={`${row.entityId}-${index}`}>
              <TableCell className="align-top">{row.entityId}</TableCell>
              <TableCell className="align-top tabular-nums">
                {row.value ?? "—"}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
