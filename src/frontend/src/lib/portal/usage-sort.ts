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
  defaultKey: K,
): UsageOrder<K> | null {
  const current = effectiveOrder(chosen, defaultKey);

  if (clicked !== current.sort) {
    return clicked === defaultKey ? null : { sort: clicked, direction: "desc" };
  }
  if (current.direction === "desc") return { sort: clicked, direction: "asc" };
  return null;
}
