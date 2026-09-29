import { useState } from "react";

import type { UsageOrder } from "@/api/usage-client";
import { effectiveOrder, nextOrder } from "@/lib/portal/usage-sort";

export function useUsageOrder<K extends string>(defaultKey: K) {
  const [chosen, setChosen] = useState<UsageOrder<K> | null>(null);

  return {
    chosen,
    shown: effectiveOrder(chosen, defaultKey),
    isDefault: chosen === null,
    toggle: (key: K) => setChosen((current) => nextOrder(current, key)),
  };
}
