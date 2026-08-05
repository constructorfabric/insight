import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";
import type {
  MetricCanonicalSelection,
  MetricDimensionFilter,
} from "@/api/metric-results-client";
import { downloadBlob } from "@/lib/download";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export interface MetricEvidenceSelection {
  metric_key: string;
  entity: { type: "person"; id: string };
  period: { from: string; to: string };
  filters: MetricDimensionFilter[];
  display_dimensions: string[];
}

export interface MetricEvidenceColumn {
  key: string;
  label: string;
  type: "string" | "number" | "date";
}

export interface MetricEvidenceRow {
  values: Record<string, unknown>;
}

export interface MetricDrilldownResponse {
  selection: MetricEvidenceSelection;
  columns: MetricEvidenceColumn[];
  rows: MetricEvidenceRow[];
  next_cursor: string | null;
}

export interface MetricDrilldownRequest extends MetricEvidenceSelection {
  cursor?: string;
  limit: number;
}

async function parseResponseJson<T>(
  res: Response,
  onInvalid: () => T
): Promise<T> {
  try {
    return (await res.json()) as T;
  } catch {
    return onInvalid();
  }
}

async function errorFor(res: Response): Promise<AnalyticsApiError> {
  const body = await parseResponseJson<unknown>(res, () => null);
  return new AnalyticsApiError(res.status, body);
}

function exportFilename(disposition: string | null, fallback: string): string {
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

export async function queryMetricDrilldown(
  request: MetricDrilldownRequest,
  signal?: AbortSignal
): Promise<MetricDrilldownResponse> {
  const res = await fetchWithAuth(`${BASE}/metric-drilldown`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(request),
    signal,
  });
  if (!res.ok) throw await errorFor(res);
  return parseResponseJson<MetricDrilldownResponse>(res, () => {
    throw new AnalyticsApiError(res.status, { error: "invalid_json" });
  });
}

export async function downloadMetricDrilldown(
  selection: MetricEvidenceSelection,
  format: "csv" | "xlsx",
  signal?: AbortSignal
): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/metric-drilldown/export`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ...selection, format }),
    signal,
  });
  if (!res.ok) throw await errorFor(res);
  const blob = await res.blob();
  downloadBlob(
    blob,
    exportFilename(
      res.headers.get("content-disposition"),
      `${selection.metric_key}.${format}`
    )
  );
}

export function evidenceSelection(
  canonical: MetricCanonicalSelection | undefined,
  entityId: string,
  period?: { from: string; to: string },
  filters?: MetricDimensionFilter[],
  displayDimensions: string[] = []
): MetricEvidenceSelection | null {
  if (!canonical) return null;
  return {
    metric_key: canonical.metric_key,
    entity: { type: "person", id: entityId },
    period: period ?? canonical.period,
    filters: filters ?? canonical.filters,
    display_dimensions: [...new Set(displayDimensions)].sort(),
  };
}
