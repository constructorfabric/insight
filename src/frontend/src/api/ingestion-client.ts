import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";
import { lookbackFrom } from "@/lib/ingestion-chart";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

/** Bucket widths the server will answer. The set is closed there too. */
export type IngestionGrain = "15m" | "1s";

/** What one plotted band counts. */
export type IngestionSeries = "connector" | "stream" | "total";

export interface IngestionPoint {
  /** Bucket start as `YYYY-MM-DD HH:MM:SS`, always UTC and never zone-marked. */
  bucket: string;
  /** Connector slug, stream name, or `all`, per the resolved `series`. */
  key: string;
  rows: number;
}

export interface IngestionIntensity {
  grain: IngestionGrain;
  series: IngestionSeries;
  /** Resolved, not as asked — the request may pin neither bound. */
  from: string;
  to: string;
  scope?: string;
  /** The server clipped the tail; the window is too wide to plot honestly. */
  truncated: boolean;
  points: IngestionPoint[];
}

export interface IngestionIntensityRequest {
  grain: IngestionGrain;
  series?: IngestionSeries;
  /** A bronze database, e.g. `bronze_bamboohr`. Null/absent = org-wide. */
  scope?: string | null;
  /** RFC 3339. Omit to let the server default from the grain. */
  from?: string;
  to?: string;
  /**
   * Ask for a window this many whole UTC days back instead of naming `from`.
   *
   * Resolved here rather than by the caller because that means reading the
   * clock, and a component may not do that during a render. Ignored when
   * `from` is given.
   */
  lookbackDays?: number;
}

async function getJson<T>(url: string): Promise<T> {
  const res = await fetchWithAuth(url);
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as T;
}

export function intensityParams(
  req: IngestionIntensityRequest,
  now: number,
): URLSearchParams {
  const params = new URLSearchParams({ grain: req.grain });
  if (req.series) params.set("series", req.series);
  if (req.scope) params.set("scope", req.scope);
  const from =
    req.from ??
    (req.lookbackDays === undefined
      ? undefined
      : lookbackFrom(now, req.lookbackDays));
  if (from) params.set("from", from);
  if (req.to) params.set("to", req.to);
  return params;
}

export async function getIngestionIntensity(
  req: IngestionIntensityRequest,
): Promise<IngestionIntensity> {
  // Inside the request, so the clock is read when the read happens rather
  // than while a component renders.
  return getJson<IngestionIntensity>(
    `${BASE}/ingestion/intensity?${intensityParams(req, Date.now())}`,
  );
}
