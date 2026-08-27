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
  entity:
    | { type: "person"; id: string }
    | { type: "persons"; ids: string[] }
    | { type: "tenant" };
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
  links?: Record<string, string>;
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

/**
 * People one drilldown may read at once. MUST match `MAX_ENTITY_PERSONS` in
 * the analytics validator: past it the request is a 400, so a caller that
 * builds one anyway trades a table for an error dialog.
 */
export const MAX_EVIDENCE_PERSONS = 1000;

/**
 * The same selection for a GROUP of people — an org or team card, whose figure
 * is taken over a roster rather than one person.
 *
 * A roster is passed as its own entity rather than one selection per member:
 * the reader asked what the number on the card is made of, and one table of
 * every record answers that where a hundred tabs do not.
 *
 * Null past the cap, so a scope too wide to read renders no affordance rather
 * than one that fails when taken. A partial table is the worse answer: it
 * would be a different figure from the one on the card, silently.
 */
export function personsEvidenceSelection(
  canonical: MetricCanonicalSelection | undefined,
  personIds: readonly string[],
  period?: { from: string; to: string },
  filters?: MetricDimensionFilter[],
  displayDimensions: string[] = []
): MetricEvidenceSelection | null {
  if (!canonical) return null;
  // Sorted and deduplicated here as well as on the server: this object is the
  // react-query key, and the same roster in another order would otherwise be a
  // second cache entry for one question.
  const ids = [...new Set(personIds)].sort();
  if (ids.length === 0 || ids.length > MAX_EVIDENCE_PERSONS) return null;

  return {
    metric_key: canonical.metric_key,
    entity: { type: "persons", ids },
    period: period ?? canonical.period,
    filters: filters ?? canonical.filters,
    display_dimensions: [...new Set(displayDimensions)].sort(),
  };
}

export function evidenceSelection(
  canonical: MetricCanonicalSelection | undefined,
  entityId?: string,
  period?: { from: string; to: string },
  filters?: MetricDimensionFilter[],
  displayDimensions: string[] = []
): MetricEvidenceSelection | null {
  if (!canonical) return null;
  const entity =
    canonical.entity.type === "tenant"
      ? ({ type: "tenant" } as const)
      : entityId
        ? ({ type: "person", id: entityId } as const)
        : null;
  if (!entity) return null;

  return {
    metric_key: canonical.metric_key,
    entity,
    period: period ?? canonical.period,
    filters: filters ?? canonical.filters,
    display_dimensions: [...new Set(displayDimensions)].sort(),
  };
}
