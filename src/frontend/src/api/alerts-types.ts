/** A comparison a number column, or the count of matching rows, takes. */
export type ComparisonOperator = ">" | ">=" | "<" | "<=" | "==" | "!=";

export type NumberOperator = ComparisonOperator | "between";

export type TextOperator =
  "==" | "!=" | "contains" | "starts_with" | "ends_with" | "in" | "not_in";

export type DateOperator =
  "before" | "after" | "between" | "older_than" | "newer_than";

export type ConditionOperator = NumberOperator | TextOperator | DateOperator;

/** Which rows must pass the test for the rule to be breached. */
export type Quantifier =
  | "one"
  | "any"
  | "all"
  | "none"
  | { count: { op: ComparisonOperator; value: number } };

/**
 * A test on one result column. `value` is a number or digit string, a text,
 * an RFC 3339 timestamp, a duration such as `P7D`, or a list of those, by
 * what the operator takes.
 */
export interface ConditionLeaf {
  column: string;
  op: ConditionOperator;
  value: unknown;
}

export type ConditionNode =
  | ConditionLeaf
  | { all: ConditionNode[] }
  | { any: ConditionNode[] }
  | { not: ConditionNode };

export interface Condition {
  rows: Quantifier;
  where: ConditionNode;
}

/**
 * The rows a check found the test held on: `columns` names each cell of a
 * row, the grouping first, then the columns the condition reads, then the
 * rule's context columns.
 */
export interface Witness {
  matched: number;
  answered: number;
  columns: string[];
  rows: unknown[][];
}

/** What one check found. Absent on an alert that has not been checked yet. */
export type AlertOutcome = "breach" | "no_breach" | "unknown";

/** Why a check could not say whether the condition was met. */
export type UnknownReason =
  | "no_rows"
  | "many_rows"
  | "column_missing"
  | "null"
  | "non_numeric"
  | "non_text"
  | "non_date"
  | "incomparable"
  | "row_limit"
  | "metric_missing"
  | "compile_failed"
  | "run_failed"
  | "timeout";

export type NotificationStatus = "pending" | "cancelled" | "sent" | "failed";

/**
 * A number as the service answers it: a JSON number while an integer is within
 * ±(2^53 − 1), its decimal digits as a string beyond that, and a draft sends
 * such a string back as it came.
 */
export type AlertNumber = number | string;

/** A rule as an administrator writes it. */
export interface AlertDraft {
  name: string;
  metric: string;
  condition: Condition;
  /** Result columns the witness keeps beside the condition's: up to 5. */
  context_columns: string[];
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
  last_witness?: Witness | null;
  last_valid_breached?: boolean;
  breached_since?: string;
}

export interface Alert {
  id: string;
  name: string;
  metric: string;
  condition: Condition;
  context_columns: string[];
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
  condition: Condition;
  /** The columns the rule asked the witness to keep beside the condition's. */
  context_columns: string[];
  witness?: Witness | null;
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

/** What a rule would find if it were checked now. */
export interface AlertPreviewRequest {
  metric: string;
  range: string | null;
  condition: Condition;
  context_columns: string[];
}

export interface AlertPreview {
  outcome: AlertOutcome;
  reason: UnknownReason | null;
  witness: Witness | null;
}
