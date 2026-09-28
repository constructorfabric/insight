import { describeAge } from "@/lib/portal/connector-health";

export function updatedLabel(
  updatedAt: string | undefined,
  now: number
): string | null {
  const at = Date.parse(updatedAt ?? "");
  if (Number.isNaN(at)) return null;

  return `Updated ${describeAge(now - at)}`;
}
