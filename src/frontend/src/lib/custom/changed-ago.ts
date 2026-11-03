import { describeAge } from "@/lib/portal/connector-health";

const DAY_MS = 24 * 60 * 60 * 1000;

/** How long ago a stored thing last changed, as a card line says it. */
export function changedAgo(updatedAt: string): string | null {
  const changed = Date.parse(updatedAt);
  if (Number.isNaN(changed)) return null;

  const age = Math.max(0, Date.now() - changed);
  if (age >= DAY_MS && age < 2 * DAY_MS) return "yesterday";

  return describeAge(age);
}
