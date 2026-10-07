import { CustomApiError } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import type { AlertField, FieldErrors } from "@/lib/alerts/draft";

const FIELDS: ReadonlySet<string> = new Set<AlertField>([
  "name",
  "metric",
  "column",
  "threshold",
  "range",
  "interval_secs",
  "destination",
]);

interface Violation {
  field?: string;
  description?: string;
}

/**
 * What the service refused a save for: each named field's reason, to show on
 * that field, and anything else to show above the form.
 */
export function savingRefusal(error: unknown): {
  fields: FieldErrors;
  general?: string;
} {
  const violations =
    error instanceof CustomApiError
      ? ((error.body as { context?: { field_violations?: Violation[] } } | null)
          ?.context?.field_violations ?? [])
      : [];
  const fields: FieldErrors = {};

  for (const { field, description } of violations) {
    if (field && description && FIELDS.has(field)) {
      fields[field as AlertField] = description;
    }
  }
  if (Object.keys(fields).length > 0) return { fields };

  return { fields, general: refusal(error, "Couldn't save the alert.") };
}
