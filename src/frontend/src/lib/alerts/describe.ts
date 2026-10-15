import type {
  NotificationStatus,
  UnknownReason,
  Witness,
} from "@/api/alerts-types";

import {
  COMPARISONS,
  OPERATORS,
  conditionParts,
  drawn,
  rowsOf,
  type WrittenLeaf,
} from "./condition";

const WHOLE = /^-?\d+$/;

const AGE_UNITS: Record<string, string> = {
  M: "minute",
  H: "hour",
  D: "day",
  W: "week",
};

function ageWords(value: unknown): string {
  const age =
    typeof value === "string" ? /^PT?(\d+)([MHDW])$/.exec(value) : null;
  if (!age) return String(value);

  const amount = Number(age[1]);
  return `${amount} ${AGE_UNITS[age[2]]}${amount === 1 ? "" : "s"}`;
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function stamp(value: unknown): string {
  const at = typeof value === "string" ? new Date(value) : undefined;
  if (at === undefined || Number.isNaN(at.getTime())) return String(value);

  return `${at.getUTCFullYear()}-${pad(at.getUTCMonth() + 1)}-${pad(at.getUTCDate())} ${pad(at.getUTCHours())}:${pad(at.getUTCMinutes())} UTC`;
}

// INVARIANT: a threshold reads exactly as its author wrote it, never rounded:
// a JSON number as JavaScript prints it, a digit string as its digits.
function thresholdWords(value: unknown): string {
  return String(value);
}

function pair(value: unknown): [unknown, unknown] {
  return Array.isArray(value) ? [value[0], value[1]] : [value, undefined];
}

function textList(value: unknown): string {
  return Array.isArray(value) ? value.map(String).join(", ") : String(value);
}

const DATE_OPERATORS: ReadonlySet<string> = new Set(
  OPERATORS.date.map(({ value }) => value)
);
const TEXT_ONLY = new Set([
  "contains",
  "starts_with",
  "ends_with",
  "in",
  "not_in",
]);

function leafWords({ column, op, value }: WrittenLeaf): string {
  if (op === "older_than" || op === "newer_than") {
    return `${column} ${op === "older_than" ? "older than" : "newer than"} ${ageWords(value)}`;
  }
  if (op === "between") {
    const [low, high] = pair(value);
    const dated = typeof low === "string" && !WHOLE.test(low);
    const words = dated ? stamp : thresholdWords;
    return `${column} between ${words(low)} and ${words(high)}`;
  }
  if (DATE_OPERATORS.has(op)) return `${column} ${op} ${stamp(value)}`;
  if (op === "in") return `${column} is one of ${textList(value)}`;
  if (op === "not_in") return `${column} is none of ${textList(value)}`;
  if (TEXT_ONLY.has(op)) {
    return `${column} ${op.replace("_", " ")} ${String(value)}`;
  }

  const label = COMPARISONS.find((option) => option.value === op)?.label ?? op;

  return `${column} ${label} ${thresholdWords(value)}`;
}

function nodeWords(value: unknown): string {
  const node = drawn(value);

  switch (node.kind) {
    case "leaf":
      return leafWords(node.leaf);
    case "not":
      return `not (${nodeWords(node.child)})`;
    case "group":
      return node.children
        .map((child) => {
          const kind = drawn(child).kind;
          return kind === "group" ? `(${nodeWords(child)})` : nodeWords(child);
        })
        .join(node.join === "all" ? " and " : " or ");
    case "unreadable":
      return JSON.stringify(value) ?? "";
  }
}

function rowsWords(rows: unknown): string {
  const read = rowsOf(rows);

  switch (read.choice) {
    case "one":
    case undefined:
      return "";
    case "any":
      return "any row: ";
    case "all":
      return "every row: ";
    case "none":
      return "no row: ";
    case "count": {
      const label =
        COMPARISONS.find((option) => option.value === read.op)?.label ??
        read.op;
      return `number of matching rows ${label} ${String(read.value)}: `;
    }
  }
}

export function conditionText(condition: unknown): string {
  const { rows, where } = conditionParts(condition);

  return `${rowsWords(rows)}${nodeWords(where)}`;
}

export function matchedText(witness: Witness): string {
  const noun = witness.answered === 1 ? "row matches" : "rows match";
  return `${witness.matched} of ${witness.answered} ${noun}`;
}

function groupedNumber(value: number): string {
  return value.toLocaleString("en-US", { maximumFractionDigits: 20 });
}

export function cellText(cell: unknown): string {
  if (cell === null || cell === undefined) return "—";
  if (typeof cell === "number") return groupedNumber(cell);

  return String(cell);
}

const REASONS: Record<UnknownReason, string> = {
  no_rows: "No rows",
  many_rows: "More than one row",
  column_missing: "Column not found",
  null: "No value",
  non_numeric: "Not a number",
  non_text: "Not text",
  non_date: "Not a date",
  incomparable: "Too large to compare exactly",
  row_limit: "Too many rows to check",
  metric_missing: "Metric not found",
  compile_failed: "Metric is invalid",
  run_failed: "Metric failed to run",
  timeout: "Metric timed out",
};

/** Why a check could not decide. A code this build does not know is shown as it came. */
export function reasonText(reason: string): string {
  return reason in REASONS ? REASONS[reason as UnknownReason] : reason;
}

const STATUSES: Record<NotificationStatus, string> = {
  pending: "Pending",
  cancelled: "Cancelled",
  sent: "Sent",
  failed: "Failed",
};

export function statusText(status: string): string {
  return status in STATUSES ? STATUSES[status as NotificationStatus] : status;
}
