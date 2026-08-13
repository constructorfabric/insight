import { AnalyticsApiError } from "@/api/analytics-client";
import { IdentityApiError } from "@/api/identity-client";

/**
 * Pull the first field-violation description out of a canonical-error body so
 * the console can show the server's own reason — the single-SELECT gate's
 * rejection or a missing-named-parameter `400` — instead of a generic message.
 * Returns the fallback when the body carries no such reason. Both backend
 * services speak the same problem+json envelope, so one extractor serves both
 * error classes.
 */
export function apiErrorReason(error: unknown, fallback: string): string {
  if (error instanceof AnalyticsApiError || error instanceof IdentityApiError) {
    const reason = firstFieldViolation(error.body);
    if (reason) return reason;
  }
  return fallback;
}

function firstFieldViolation(body: unknown): string | null {
  if (typeof body !== "object" || body === null) return null;
  const context = (body as { context?: unknown }).context;
  if (typeof context !== "object" || context === null) return null;
  const violations = (context as { field_violations?: unknown })
    .field_violations;
  if (!Array.isArray(violations) || violations.length === 0) return null;
  const description = (violations[0] as { description?: unknown }).description;
  return typeof description === "string" ? description : null;
}
