import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";
import type { MetricFormat } from "@/api/metric-results-client";
import { downloadBlob } from "@/lib/download";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export const MAX_REPORT_PEOPLE = 1000;

export type ReportSubject =
  { type: "people"; ids: string[] } | { type: "tenant" };
export type ReportGranularity = "day" | "week" | "month" | "quarter" | "year";
export type ReportExportFormat = "csv" | "xlsx";

export interface ReportRecipe {
  subject: ReportSubject;
  period: { from: string; to: string };
  granularity: ReportGranularity;
  metric_keys: string[];
}

export interface ReportColumn {
  key: string;
  label: string;
  data_type: "text" | "date" | "number";
  format?: MetricFormat;
  unit?: string;
}

export interface ReportPreviewResponse {
  columns: ReportColumn[];
  rows: Array<Array<string | number | null>>;
  total_rows: number;
}

function reportFilename(disposition: string | null, fallback: string): string {
  const encoded = disposition?.match(/filename\*=UTF-8''([^;]+)/i)?.[1];
  if (encoded) {
    try {
      return decodeURIComponent(encoded);
    } catch {
      return disposition?.match(/filename="?([^";]+)"?/i)?.[1] ?? fallback;
    }
  }
  return disposition?.match(/filename="?([^";]+)"?/i)?.[1] ?? fallback;
}

async function errorFor(response: Response): Promise<AnalyticsApiError> {
  const body = await response.json().catch(() => null);
  return new AnalyticsApiError(response.status, body);
}

export async function previewReport(
  recipe: ReportRecipe,
  signal?: AbortSignal
): Promise<ReportPreviewResponse> {
  const response = await fetchWithAuth(`${BASE}/reports/preview`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(recipe),
    signal,
  });
  if (!response.ok) throw await errorFor(response);

  try {
    return (await response.json()) as ReportPreviewResponse;
  } catch {
    throw new AnalyticsApiError(response.status, { error: "invalid_json" });
  }
}

export async function downloadReport(
  recipe: ReportRecipe,
  format: ReportExportFormat,
  signal?: AbortSignal
): Promise<void> {
  const response = await fetchWithAuth(`${BASE}/reports/export`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...recipe, format }),
    signal,
  });
  if (!response.ok) throw await errorFor(response);

  downloadBlob(
    await response.blob(),
    reportFilename(
      response.headers.get("content-disposition"),
      `report.${format}`
    )
  );
}
