import type { UsageRange } from "@/api/usage-client";

export function keepWithinPeriod<T extends UsageRange>(range: UsageRange) {
  return (previous: T | undefined): T | undefined =>
    previous?.since === range.since && previous.until === range.until ? previous : undefined;
}
