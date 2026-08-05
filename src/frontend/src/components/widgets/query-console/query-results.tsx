import { useTranslation } from "react-i18next";

import {
  BarChart,
  CartesianGrid,
  ChartBar,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
  XAxis,
  YAxis,
  type ChartConfig,
} from "@/components/ui/chart";
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
import { seriesColors } from "@/lib/series-colors";
import {
  formatCell,
  inferChartModel,
  inferColumns,
  toNumber,
  type ResultRow,
} from "@/lib/query-console/result-shape";
import { TableIcon } from "lucide-react";

export function QueryResults({ rows }: { rows: ResultRow[] }) {
  const { t } = useTranslation();

  if (rows.length === 0) {
    return (
      <Empty className="min-h-40">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <TableIcon />
          </EmptyMedia>
          <EmptyTitle>{t("query_console.results.empty")}</EmptyTitle>
        </EmptyHeader>
      </Empty>
    );
  }

  return (
    <div className="flex flex-col gap-6">
      <QueryAutoChart rows={rows} />
      <ResultTable rows={rows} />
    </div>
  );
}

function QueryAutoChart({ rows }: { rows: ResultRow[] }) {
  const model = inferChartModel(rows);
  if (!model) return null;

  // Column aliases are arbitrary SQL (e.g. `total sales`), which would produce
  // an invalid `var(--color-total sales)`. Key each series by a stable
  // `series-<n>` instead and keep the column name only as the display label.
  const seriesKeys = model.valueKeys.map((_, index) => `series-${index}`);
  const colors = seriesColors(seriesKeys);
  const config: ChartConfig = Object.fromEntries(
    model.valueKeys.map((column, index) => [
      seriesKeys[index],
      { label: column, color: colors[seriesKeys[index]] },
    ])
  );
  const data = rows.map((row) => {
    // Keep a missing value as null (a gap), not 0 — a bar at zero would report
    // a measured zero the query never returned.
    const point: Record<string, string | number | null> = {
      label: formatCell(row[model.labelKey]),
    };
    model.valueKeys.forEach((column, index) => {
      point[seriesKeys[index]] = toNumber(row[column]);
    });
    return point;
  });

  return (
    <ChartContainer config={config} className="aspect-auto h-64 w-full">
      <BarChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
        <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
        <XAxis
          dataKey="label"
          tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
          tickLine={false}
          axisLine={false}
          height={24}
          interval="preserveStartEnd"
        />
        <YAxis
          tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
          tickLine={false}
          axisLine={false}
          width={48}
        />
        <ChartTooltip content={<ChartTooltipContent className="min-w-40" />} />
        {model.valueKeys.map((column, index) => (
          <ChartBar
            key={seriesKeys[index]}
            dataKey={seriesKeys[index]}
            fill={`var(--color-${seriesKeys[index]})`}
            name={column}
            radius={[2, 2, 0, 0]}
          />
        ))}
      </BarChart>
    </ChartContainer>
  );
}

function ResultTable({ rows }: { rows: ResultRow[] }) {
  const columns = inferColumns(rows);

  return (
    <div className="overflow-x-auto">
      <Table>
        <TableHeader>
          <TableRow>
            {columns.map((column) => (
              <TableHead key={column} className="font-mono text-xs">
                {column}
              </TableHead>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row, index) => (
            <TableRow key={index}>
              {columns.map((column) => (
                <TableCell key={column} className="align-top tabular-nums">
                  {formatCell(row[column])}
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}
