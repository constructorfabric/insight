import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

export interface UsageConfig {
  enabled: boolean;
}

export interface UsageTotals {
  visits: number;
  visitors: number;
  page_views: number;
}

export interface UsageDay {
  day: string;
  visits: number;
  visitors: number;
}

export interface UsagePerson {
  person_id: string;
  display_name: string;
  username: string;
  visits: number;
  page_views: number;
  last_seen: string;
}

export interface UsagePage {
  path: string;
  views: number;
  visitors: number;
}

export interface UsageEvent {
  event_name: string;
  target: string;
  opens: number;
  people: number;
}

export interface UsageSummary {
  since: string;
  until: string;
  totals: UsageTotals;
  by_day: UsageDay[];
}

export interface UsageList<T> {
  since: string;
  until: string;
  items: T[];
}

export type SortDirection = "asc" | "desc";

export interface UsageOrder<K extends string> {
  sort: K;
  direction: SortDirection;
}

export type UsagePeopleSort = "visits" | "page_views" | "last_seen";
export type UsagePagesSort = "views" | "visitors";
export type UsageActionsSort = "opens" | "people";

export interface UsageRange {
  since: string;
  until: string;
}

async function getJson<T>(url: string): Promise<T> {
  const res = await fetchWithAuth(url);
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as T;
}

export async function getUsageConfig(): Promise<UsageConfig> {
  return getJson<UsageConfig>(`${BASE}/usage/config`);
}

export async function getUsageSummary(range: UsageRange): Promise<UsageSummary> {
  const params = new URLSearchParams({ since: range.since, until: range.until });
  return getJson<UsageSummary>(`${BASE}/usage/summary?${params}`);
}

function listUrl<K extends string>(
  list: string,
  range: UsageRange,
  order: UsageOrder<K> | null,
): string {
  const params = new URLSearchParams({ since: range.since, until: range.until });
  if (order) {
    params.set("sort", order.sort);
    params.set("direction", order.direction);
  }
  return `${BASE}/usage/${list}?${params}`;
}

export async function getUsagePeople(
  range: UsageRange,
  order: UsageOrder<UsagePeopleSort> | null,
): Promise<UsageList<UsagePerson>> {
  return getJson<UsageList<UsagePerson>>(listUrl("people", range, order));
}

export async function getUsagePages(
  range: UsageRange,
  order: UsageOrder<UsagePagesSort> | null,
): Promise<UsageList<UsagePage>> {
  return getJson<UsageList<UsagePage>>(listUrl("pages", range, order));
}

export async function getUsageActions(
  range: UsageRange,
  order: UsageOrder<UsageActionsSort> | null,
): Promise<UsageList<UsageEvent>> {
  return getJson<UsageList<UsageEvent>>(listUrl("actions", range, order));
}
