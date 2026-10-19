import { CustomApiError } from "@/api/custom-client";
import { refusal } from "@/components/custom/refusal";
import type { FieldErrors } from "@/lib/alerts/draft";

/** The draft's own fields; a condition's refusals name a path inside it. */
const FIELDS: ReadonlySet<string> = new Set([
  "name",
  "metric",
  "range",
  "interval_secs",
  "destination",
  "enabled",
]);

function aboutTheDraft(field: string): boolean {
  return (
    FIELDS.has(field) || field === "condition" || field.startsWith("condition.")
  );
}

interface Violation {
  field?: string;
  description?: string;
}

/**
 * What the service refused a draft for: each named field's reason, keyed by
 * the path it names, to show where that is, and anything else to show above
 * the form.
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
  const fields: Record<string, string> = {};

  for (const { field, description } of violations) {
    if (field && description && aboutTheDraft(field)) {
      fields[field] = description;
    }
  }
  if (Object.keys(fields).length > 0) return { fields };

  return { fields, general: refusal(error, "Couldn't save the alert.") };
}
