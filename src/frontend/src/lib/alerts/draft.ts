/** An alert as the form holds it, and the rule it sends. */

import type { Alert, AlertDraft, AlertOperator } from "@/api/alerts-types";
import {
  fromSeconds,
  toSeconds,
  type IntervalInput,
} from "@/lib/alerts/interval";

/** The longest name the service accepts. */
export const NAME_MAX = 200;

export interface AlertForm {
  name: string;
  metric: string;
  column: string;
  operator: AlertOperator;
  /** As typed, so a half-written number is not lost while the reader types. */
  threshold: string;
  /** A window token, or "" to run the metric over all time. */
  range: string;
  interval: IntervalInput;
  /**
   * The interval as stored, when the units above cannot hold it exactly
   * (90 seconds is shown as 2 minutes). Sent back unchanged unless the
   * reader changes the interval.
   */
  storedIntervalSecs?: number;
  destination: string;
  enabled: boolean;
}

/** A field the form can say something about, named as the service names it. */
export type AlertField =
  | "name"
  | "metric"
  | "column"
  | "threshold"
  | "range"
  | "interval_secs"
  | "destination";

export type FieldErrors = Partial<Record<AlertField, string>>;

export function blankForm(destination = ""): AlertForm {
  return {
    name: "",
    metric: "",
    column: "",
    operator: ">",
    threshold: "",
    range: "",
    interval: { amount: 5, unit: "minutes" },
    destination,
    enabled: true,
  };
}

export function formOf(alert: Alert): AlertForm {
  return {
    name: alert.name,
    metric: alert.metric,
    column: alert.column,
    operator: alert.operator,
    threshold: String(alert.threshold),
    range: alert.range ?? "",
    interval: fromSeconds(alert.interval_secs),
    storedIntervalSecs: alert.interval_secs,
    destination: alert.destination,
    enabled: alert.enabled,
  };
}

/**
 * The threshold as a number, or nothing while it is not one that can be sent
 * exactly: past 2^53 a JavaScript number no longer holds every integer.
 */
export function thresholdOf(typed: string): number | undefined {
  const trimmed = typed.trim();
  if (trimmed === "") return undefined;

  const value = Number(trimmed);
  if (!Number.isFinite(value)) return undefined;
  if (Number.isInteger(value) && !Number.isSafeInteger(value)) return undefined;

  return value;
}

function thresholdError(typed: string): string {
  const value = Number(typed.trim());

  return Number.isInteger(value) && !Number.isSafeInteger(value)
    ? "This number is too large to enter here exactly."
    : "Enter a number.";
}

/** The stored interval while the reader has not changed it, else what they entered. */
function intervalSecsOf(form: AlertForm): number {
  const stored = form.storedIntervalSecs;
  if (stored === undefined) return toSeconds(form.interval);

  const shown = fromSeconds(stored);
  const untouched =
    shown.amount === form.interval.amount && shown.unit === form.interval.unit;

  return untouched ? stored : toSeconds(form.interval);
}

export type Checked =
  { ok: true; draft: AlertDraft } | { ok: false; errors: FieldErrors };

/**
 * The rule the form says, or what is wrong with it.
 *
 * Only what the form alone can know is checked here. The installation's
 * bounds (the shortest interval, a window the metric allows) belong to the
 * service, and its refusal is shown on the field it names.
 */
export function checkForm(form: AlertForm): Checked {
  const errors: FieldErrors = {};
  const name = form.name.trim();
  // INVARIANT: the service counts characters, and `length` counts UTF-16 units.
  const nameLength = [...name].length;
  const threshold = thresholdOf(form.threshold);
  const amount = form.interval.amount;

  if (name === "") errors.name = "Give the alert a name.";
  else if (nameLength > NAME_MAX) {
    errors.name = `Keep the name to ${NAME_MAX} characters.`;
  }
  if (form.metric === "") errors.metric = "Pick the metric to watch.";
  if (form.column === "")
    errors.column = "Pick the column that holds the number.";
  if (threshold === undefined)
    errors.threshold = thresholdError(form.threshold);
  if (!Number.isInteger(amount) || amount < 1) {
    errors.interval_secs = "Enter a whole number, 1 or more.";
  }
  if (form.destination === "") errors.destination = "Pick where to send it.";

  if (Object.keys(errors).length > 0 || threshold === undefined) {
    return { ok: false, errors };
  }

  return {
    ok: true,
    draft: {
      name,
      metric: form.metric,
      column: form.column,
      operator: form.operator,
      threshold,
      range: form.range === "" ? null : form.range,
      interval_secs: intervalSecsOf(form),
      destination: form.destination,
      enabled: form.enabled,
    },
  };
}
