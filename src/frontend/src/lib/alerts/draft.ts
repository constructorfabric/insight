import type {
  Alert,
  AlertDraft,
  AlertNumber,
  Condition,
} from "@/api/alerts-types";
import type { Held } from "@/lib/custom/editor/document";

import {
  blankLeaf,
  conditionParts,
  fits,
  leavesOf,
  rowsOf,
  valueShape,
  type ColumnKind,
} from "./condition";

const NAME_MAX = 200;
const DEFAULT_INTERVAL_SECS = 300;
/** The most context columns the service takes on a rule. */
export const CONTEXT_COLUMNS_MAX = 5;

export type AlertDocument = Record<string, unknown>;

/** Keyed by the path the service names a refusal by, as `condition.where.all[1].value`. */
export type FieldErrors = Readonly<Record<string, string>>;

export function blankForm(destination = ""): AlertDocument {
  return {
    name: "",
    metric: "",
    condition: { rows: "one", where: blankLeaf() },
    context_columns: [],
    range: null,
    interval_secs: DEFAULT_INTERVAL_SECS,
    destination,
    enabled: true,
  };
}

export function formOf(alert: Alert): AlertDocument {
  return {
    name: alert.name,
    metric: alert.metric,
    condition: alert.condition,
    context_columns: alert.context_columns,
    range: alert.range ?? null,
    interval_secs: alert.interval_secs,
    destination: alert.destination,
    enabled: alert.enabled,
  };
}

export function textOf(document: AlertDocument, key: string): string {
  const value = document[key];
  return typeof value === "string" ? value : "";
}

/** The context columns the document names, or nothing while it names them badly. */
export function contextColumnsOf(
  document: AlertDocument
): string[] | undefined {
  const value = document.context_columns;
  if (value === undefined) return [];
  if (!Array.isArray(value)) return undefined;
  if (!value.every((column) => typeof column === "string")) return undefined;

  return value as string[];
}

const DECIMAL = /^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i;
const DIGITS = /^-?\d+$/;
const MOMENT =
  /^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;
const AGE = /^P(T[1-9]\d*[HM]|[1-9]\d*[DW])$/;

// INVARIANT: past 2^53 a JavaScript number no longer holds every integer, so
// a wider one travels as its digits, as the service writes it.
export function numberOf(typed: string): AlertNumber | undefined {
  const trimmed = typed.trim();
  if (!DECIMAL.test(trimmed)) return undefined;

  const value = Number(trimmed);
  if (!Number.isFinite(value)) return undefined;
  if (Number.isSafeInteger(value) || !Number.isInteger(value)) return value;

  return DIGITS.test(trimmed) ? trimmed : undefined;
}

function isNumber(value: unknown): boolean {
  if (typeof value === "number") return Number.isFinite(value);
  return typeof value === "string" && DIGITS.test(value);
}

function isMoment(value: unknown): boolean {
  return (
    typeof value === "string" &&
    MOMENT.test(value) &&
    !Number.isNaN(Date.parse(value))
  );
}

const NUMBER = "Enter a number.";
const MOMENT_TEXT = "Pick a day.";

function valueErrors(
  shape: ReturnType<typeof valueShape>,
  value: unknown
): Record<string, string> {
  const [low, high] = Array.isArray(value) ? value : [];
  const pair = (valid: (end: unknown) => boolean, said: string) => ({
    ...(valid(low) ? {} : { "value[0]": said }),
    ...(valid(high) ? {} : { "value[1]": said }),
  });

  switch (shape) {
    case "number":
      return isNumber(value) ? {} : { value: NUMBER };
    case "number-pair":
      return pair(isNumber, NUMBER);
    case "moment":
      return isMoment(value) ? {} : { value: MOMENT_TEXT };
    case "moment-pair":
      return pair(isMoment, MOMENT_TEXT);
    case "text-list":
      return Array.isArray(value) && value.length > 0
        ? {}
        : { value: "Enter at least one value." };
    case "age":
      return typeof value === "string" && AGE.test(value)
        ? {}
        : { value: "Enter a whole number above 0." };
    case "text":
      return typeof value === "string" ? {} : { value: "Enter text." };
  }
}

function conditionErrors(
  condition: unknown,
  kinds: ReadonlyMap<string, ColumnKind>
): Record<string, string> {
  const errors: Record<string, string> = {};
  const rows = rowsOf(conditionParts(condition).rows);
  if (
    rows.choice === "count" &&
    !(Number.isSafeInteger(rows.value) && (rows.value as number) >= 0)
  ) {
    errors["condition.rows.count.value"] = "Enter a whole number of rows.";
  }

  for (const { path, leaf } of leavesOf(condition)) {
    const at = `condition.${path}`;
    const kind = kinds.get(leaf.column);
    if (leaf.column === "") {
      errors[`${at}.column`] = "Pick a column.";
      continue;
    }
    if (kind === undefined) continue;
    if (!fits(kind, leaf.op)) {
      errors[`${at}.op`] = "Not for this column.";
      continue;
    }

    const found = valueErrors(valueShape(kind, leaf.op), leaf.value);
    for (const [part, said] of Object.entries(found)) {
      errors[`${at}.${part}`] = said;
    }
  }

  return errors;
}

type Reviver = (
  key: string,
  value: unknown,
  context?: { source?: string }
) => unknown;

type Parse = (text: string, reviver: Reviver) => unknown;

/**
 * SAFETY: an integer literal past 2^53 keeps its digits rather than being
 * rounded; where the parser cannot show a literal's source, it is refused.
 */
export function parseDraftText(
  text: string,
  parse: Parse = JSON.parse as Parse
): { ok: true; value: unknown } | { ok: false; problem: string } {
  let blind = false;
  const exact: Reviver = (_key, value, context) => {
    if (typeof value !== "number" || Number.isSafeInteger(value)) return value;
    if (!Number.isInteger(value)) return value;
    if (context?.source === undefined) {
      blind = true;
      return value;
    }
    return DIGITS.test(context.source) ? context.source : value;
  };

  try {
    const value = parse(text, exact);
    return blind
      ? { ok: false, problem: "Put integers this large in quotes." }
      : { ok: true, value };
  } catch (error) {
    return { ok: false, problem: (error as Error).message };
  }
}

export type Checked =
  { ok: true; draft: AlertDraft } | { ok: false; errors: FieldErrors };

export function checkForm(
  document: AlertDocument,
  kinds: ReadonlyMap<string, ColumnKind> = new Map()
): Checked {
  const errors: Record<string, string> = {};
  const name = textOf(document, "name").trim();
  // INVARIANT: the service counts characters, and `length` counts UTF-16 units.
  const nameLength = [...name].length;
  const metric = textOf(document, "metric");
  const destination = textOf(document, "destination");
  const contextColumns = contextColumnsOf(document);
  const { range, interval_secs: interval, enabled, condition } = document;

  if (name === "") errors.name = "Enter a name.";
  else if (nameLength > NAME_MAX) {
    errors.name = `Keep the name to ${NAME_MAX} characters.`;
  }
  if (metric === "") errors.metric = "Pick a metric.";
  if (range !== null && range !== undefined && typeof range !== "string") {
    errors.range = "Pick a window.";
  }
  if (!Number.isSafeInteger(interval) || (interval as number) <= 0) {
    errors.interval_secs = "Enter a whole number of seconds.";
  }
  if (destination === "") errors.destination = "Pick a destination.";
  if (typeof enabled !== "boolean") errors.enabled = "Enter true or false.";
  if (contextColumns === undefined) {
    errors.context_columns = "Enter a list of column names.";
  } else if (contextColumns.length > CONTEXT_COLUMNS_MAX) {
    errors.context_columns = `Pick at most ${CONTEXT_COLUMNS_MAX} columns.`;
  }
  if (metric !== "") Object.assign(errors, conditionErrors(condition, kinds));

  if (Object.keys(errors).length > 0 || contextColumns === undefined) {
    return { ok: false, errors };
  }

  return {
    ok: true,
    draft: {
      name,
      metric,
      condition: condition as Condition,
      context_columns: contextColumns,
      range: typeof range === "string" && range !== "" ? range : null,
      interval_secs: interval as number,
      destination,
      enabled: enabled as boolean,
    },
  };
}

export function retypeDraft(held: Held, text: string): Held {
  const read = parseDraftText(text);
  if (!read.ok) return { ...held, text, unparsed: read.problem };

  const { value } = read;
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return { ...held, text, unparsed: "An alert is an object." };
  }

  return { document: value as AlertDocument, text };
}
