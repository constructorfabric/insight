/** An alert as the form holds it, and the rule it sends. */

import type { Alert, AlertDraft, AlertOperator } from "@/api/alerts-types";

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
  /** How often to check, in seconds. */
  intervalSecs: number;
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
    intervalSecs: 300,
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
    intervalSecs: alert.interval_secs,
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
    ? "Number is too large."
    : "Enter a number.";
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

  if (name === "") errors.name = "Enter a name.";
  else if (nameLength > NAME_MAX) {
    errors.name = `Keep the name to ${NAME_MAX} characters.`;
  }
  if (form.metric === "") errors.metric = "Pick a metric.";
  if (form.column === "") errors.column = "Pick a column.";
  if (threshold === undefined)
    errors.threshold = thresholdError(form.threshold);
  if (form.destination === "") errors.destination = "Pick a destination.";

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
      interval_secs: form.intervalSecs,
      destination: form.destination,
      enabled: form.enabled,
    },
  };
}
