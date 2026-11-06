import { AnalyticsApiError } from "@/api/analytics-client";

export interface Violation {
  field?: string;
  description: string;
}

export function violations(error: AnalyticsApiError): Violation[] {
  const body = error.body as {
    context?: { field_violations?: Violation[] };
  } | null;
  return (
    body?.context?.field_violations?.filter(
      (violation) => violation.description
    ) ?? []
  );
}

export function rowIndex(violation: Violation): number | null {
  if (/expressions\s+\d+.*\band\s+\d+.*\bstart/i.test(violation.description))
    return null;
  const match = /^crons\[(\d+)\]$/.exec(violation.field ?? "");
  return match ? Number(match[1]) : null;
}
