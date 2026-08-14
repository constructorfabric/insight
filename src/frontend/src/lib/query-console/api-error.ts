import { AnalyticsApiError } from "@/api/analytics-client";
import { IdentityApiError } from "@/api/identity-client";

/**
 * Pull the server's own reason out of a canonical-error body so the console
 * can show it — the single-SELECT gate's rejection, a missing-named-parameter
 * `400`, or a `permission_denied` explanation — instead of a generic message.
 * A `400` carries it in `context.field_violations`; a `403` (and other
 * non-field refusals) in `context.reason`. Returns the fallback when the body
 * carries neither. Both backend services speak the same problem+json
 * envelope, so one extractor serves both error classes.
 */
export function apiErrorReason(error: unknown, fallback: string): string {
  if (error instanceof AnalyticsApiError || error instanceof IdentityApiError) {
    const reason =
      firstFieldViolation(error.body) ?? contextReason(error.body);
    if (reason) return reason;
  }
  return fallback;
}

function firstFieldViolation(body: unknown): string | null {
  const context = errorContext(body);
  if (!context) return null;
  const violations = (context as { field_violations?: unknown })
    .field_violations;
  if (!Array.isArray(violations) || violations.length === 0) return null;
  const description = (violations[0] as { description?: unknown }).description;
  return typeof description === "string" ? description : null;
}

function contextReason(body: unknown): string | null {
  const context = errorContext(body);
  if (!context) return null;
  const reason = (context as { reason?: unknown }).reason;
  return typeof reason === "string" && reason.trim() !== "" ? reason : null;
}

function errorContext(body: unknown): object | null {
  if (typeof body !== "object" || body === null) return null;
  const context = (body as { context?: unknown }).context;
  return typeof context === "object" && context !== null ? context : null;
}
