export type MetricTimeseriesTableTone =
  | "default"
  | "muted"
  | "success"
  | "destructive";

type MetricTimeseriesTableTemplatePart =
  | {
      metric: string;
      prefix?: string;
      tone?: MetricTimeseriesTableTone;
    }
  | { text: string };

export type MetricTimeseriesTableColumnConfig =
  | { metric: string; labelSource?: "short" }
  | {
      label: string;
      template: readonly MetricTimeseriesTableTemplatePart[];
    };

export interface MetricTimeseriesTableConfig {
  columns: readonly MetricTimeseriesTableColumnConfig[];
}
