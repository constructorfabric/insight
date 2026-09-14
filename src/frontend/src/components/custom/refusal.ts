import { CustomApiError } from "@/api/custom-client";

/**
 * What the service said, when it said anything readable.
 *
 * A canonical error carries a precondition violation under
 * `context.violations` and a plain refusal under `detail` — both read off live
 * responses, not guessed at.
 */
export function refusal(error: unknown, fallback: string): string {
  if (error instanceof CustomApiError) {
    const body = error.body as
      | {
          detail?: string;
          context?: { violations?: { description?: string }[] };
        }
      | null;

    return (
      body?.context?.violations?.[0]?.description ?? body?.detail ?? fallback
    );
  }

  return fallback;
}
