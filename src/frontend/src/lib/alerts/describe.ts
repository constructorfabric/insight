/** How an alert's rule and what its checks found read in words. */

import type {
  AlertNumber,
  AlertOperator,
  NotificationStatus,
  UnknownReason,
} from "@/api/alerts-types";

export const OPERATORS: readonly { value: AlertOperator; label: string }[] = [
  { value: ">", label: "above" },
  { value: ">=", label: "at or above" },
  { value: "<", label: "below" },
  { value: "<=", label: "at or below" },
];

const OPERATOR_WORDS = new Map(
  OPERATORS.map(({ value, label }) => [value, label])
);

/** "above 10", "at or below 0.5". */
export function conditionText(
  operator: AlertOperator,
  threshold: AlertNumber
): string {
  return `${OPERATOR_WORDS.get(operator) ?? operator} ${numberText(threshold)}`;
}

/**
 * A number as the reader is shown it, grouped. Digits the service sent as a
 * string are grouped as digits, since turning them into a number would round
 * them.
 */
export function numberText(value: AlertNumber): string {
  if (typeof value === "string") {
    return /^-?\d+$/.test(value)
      ? value.replace(/\B(?=(\d{3})+(?!\d))/g, ",")
      : value;
  }

  return value.toLocaleString("en-US", { maximumFractionDigits: 20 });
}

const REASONS: Record<UnknownReason, string> = {
  no_rows: "The metric answered no rows.",
  many_rows:
    "The metric answered more than one row; an alert reads exactly one.",
  column_missing: "The metric has no such column.",
  null: "The value was empty.",
  non_numeric: "The value is not a number.",
  incomparable:
    "The value is too large to compare with a fractional threshold exactly.",
  metric_missing: "The metric no longer exists.",
  compile_failed: "The metric could not be compiled.",
  run_failed: "The metric failed to run.",
  timeout: "The metric took too long to answer.",
};

/** Why a check could not decide. A code this build does not know is shown as it came. */
export function reasonText(reason: string): string {
  return reason in REASONS ? REASONS[reason as UnknownReason] : reason;
}

const STATUSES: Record<NotificationStatus, string> = {
  pending: "Pending",
  cancelled: "Withdrawn",
  sent: "Sent",
  failed: "Failed",
};

export function statusText(status: string): string {
  return status in STATUSES ? STATUSES[status as NotificationStatus] : status;
}
