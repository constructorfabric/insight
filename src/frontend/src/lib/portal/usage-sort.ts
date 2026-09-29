import type { UsageOrder } from "@/api/usage-client";

export function effectiveOrder<K extends string>(
  chosen: UsageOrder<K> | null,
  defaultKey: K,
): UsageOrder<K> {
  return chosen ?? { sort: defaultKey, direction: "desc" };
}

export function nextOrder<K extends string>(
  chosen: UsageOrder<K> | null,
  clicked: K,
): UsageOrder<K> | null {
  if (chosen?.sort !== clicked) return { sort: clicked, direction: "desc" };
  if (chosen.direction === "desc") return { sort: clicked, direction: "asc" };
  return null;
}
