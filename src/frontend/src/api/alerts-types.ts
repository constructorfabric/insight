/** How an observed value is compared with an alert's threshold. */
export type AlertOperator = ">" | ">=" | "<" | "<=";

/** What one check found. Absent on an alert that has not been checked yet. */
export type AlertOutcome = "breach" | "no_breach" | "unknown";

/** Why a check could not say whether the condition was met. */
export type UnknownReason =
  | "no_rows"
  | "many_rows"
  | "column_missing"
  | "null"
  | "non_numeric"
  | "incomparable"
  | "metric_missing"
  | "compile_failed"
  | "run_failed"
  | "timeout";

export type NotificationStatus = "pending" | "cancelled" | "sent" | "failed";

/**
 * A number as the service answers it. An integer wider than 64 bits arrives as
 * its decimal digits; one between 2^53 and 2^63 arrives as a JSON number, which
 * JSON parsing rounds.
 */
export type AlertNumber = number | string;

/** A rule as an administrator writes it. */
export interface AlertDraft {
  name: string;
  metric: string;
  column: string;
  operator: AlertOperator;
  threshold: number;
  range?: string | null;
  interval_secs: number;
  destination: string;
  enabled: boolean;
  /** The revision a replacement expects; sent only when changing an alert. */
  expected_revision?: number;
}

/** What the latest checks left on the rule. */
export interface AlertState {
  last_evaluated_at?: string;
  last_outcome?: AlertOutcome;
  last_reason?: UnknownReason;
  last_value?: AlertNumber;
  last_valid_breached?: boolean;
  breached_since?: string;
}

export interface Alert {
  id: string;
  name: string;
  metric: string;
  column: string;
  operator: AlertOperator;
  threshold: AlertNumber;
  range?: string | null;
  interval_secs: number;
  destination: string;
  enabled: boolean;
  revision: number;
  state: AlertState;
  created_at: string;
  updated_at: string;
}

export interface AlertSummary {
  id: string;
  name: string;
  metric: string;
  enabled: boolean;
}

export interface AlertPage {
  alerts: AlertSummary[];
  total: number;
  limit: number;
  offset: number;
}

export interface AlertNotification {
  id: string;
  rule_revision: number;
  metric: string;
  column: string;
  operator: AlertOperator;
  threshold: AlertNumber;
  value: AlertNumber;
  evaluated_at: string;
  destination: string;
  status: NotificationStatus;
  attempts: number;
  last_error?: string | null;
  provider_receipt?: string | null;
  created_at: string;
}

export interface NotificationPage {
  notifications: AlertNotification[];
  limit: number;
  offset: number;
}

export interface AlertDestination {
  name: string;
  provider: string;
}
