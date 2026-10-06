import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";

import type {
  Alert,
  AlertNotification,
  AlertSummary,
} from "@/api/alerts-types";

/** A fresh client per render, so no test reads another's cache. */
export function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

export const ALERT: Alert = {
  id: "a1",
  name: "Too many open PRs",
  metric: "prs-open",
  column: "total",
  operator: ">",
  threshold: 10,
  range: null,
  interval_secs: 300,
  destination: "ops",
  enabled: true,
  revision: 3,
  state: {},
  created_at: "2026-10-01T08:00:00+00:00",
  updated_at: "2026-10-01T08:00:00+00:00",
};

/** The alert as the list reads it. */
export const SUMMARY: AlertSummary = {
  id: ALERT.id,
  name: ALERT.name,
  metric: ALERT.metric,
  enabled: ALERT.enabled,
};

/** A notification the alert owed under its current rule, and delivered. */
export const NOTIFICATION: AlertNotification = {
  id: "n1",
  rule_revision: ALERT.revision,
  metric: ALERT.metric,
  column: ALERT.column,
  operator: ALERT.operator,
  threshold: ALERT.threshold,
  value: 12,
  evaluated_at: "2026-10-01T09:00:00+00:00",
  destination: ALERT.destination,
  status: "sent",
  attempts: 1,
  provider_receipt: "msg-7",
  created_at: "2026-10-01T09:00:00+00:00",
};
