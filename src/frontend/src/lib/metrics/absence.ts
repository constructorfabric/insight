import type { PersonAbsenceContext } from "@/api/metric-results-client";

export function absenceLabel(context: PersonAbsenceContext | undefined): string | null {
  if (context?.period_overlap && context.compare_to_overlap) return "Time off in both periods";
  if (context?.period_overlap) return "Time off in this period";
  if (context?.compare_to_overlap) return "Time off in previous period";
  return null;
}
