import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

const JSON_HEADERS = { "Content-Type": "application/json" };

/** The service's own budget for one submission; past it the write is refused. */
export const FEEDBACK_MESSAGE_MAX = 4000;

export interface FeedbackSubmission {
  message: string;
  path: string;
  app_name: string;
  app_version: string;
}

export interface FeedbackEntry {
  feedback_id: string;
  ts: string;
  person_id: string;
  display_name: string;
  username: string;
  message: string;
  path: string;
}

export interface FeedbackList {
  since: string;
  until: string;
  items: FeedbackEntry[];
}

export interface FeedbackRange {
  since: string;
  until: string;
}

export async function submitFeedback(body: FeedbackSubmission): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/feedback`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
}

export async function getFeedback(range: FeedbackRange): Promise<FeedbackList> {
  const params = new URLSearchParams({ since: range.since, until: range.until });
  const res = await fetchWithAuth(`${BASE}/feedback?${params}`);
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as FeedbackList;
}
