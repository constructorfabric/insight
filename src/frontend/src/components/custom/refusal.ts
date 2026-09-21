import { CustomApiError } from "@/api/custom-client";

interface Said {
  description?: string;
}

interface Refusal {
  detail?: string;
  context?: { field_violations?: Said[]; violations?: Said[] };
}

/**
 * What the service said, when it said anything readable.
 *
 * A 400 names places in the document under `field_violations`, a 409 names
 * things outside it under `violations`, and both carry a plain `detail`.
 */
export function refusal(error: unknown, fallback: string): string {
  if (!(error instanceof CustomApiError)) return fallback;

  const body = error.body as Refusal | null;
  const said =
    body?.context?.field_violations?.[0]?.description ??
    body?.context?.violations?.[0]?.description;

  return said ?? body?.detail ?? fallback;
}
