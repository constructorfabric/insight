import { CustomApiError } from "@/api/custom-client";

/**
 * Whether an alert moved on since it was read.
 *
 * A 409 alone does not say so: reaching the installation's alert limit is
 * answered 409 too. The violation's type is what names a revision.
 */
export function isRevisionConflict(error: unknown): boolean {
  if (!(error instanceof CustomApiError) || error.status !== 409) return false;

  const violations =
    (error.body as { context?: { violations?: { type?: string }[] } } | null)
      ?.context?.violations ?? [];

  return violations.some((violation) => violation.type === "revision");
}
