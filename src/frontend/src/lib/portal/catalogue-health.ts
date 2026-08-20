import type { MetricDefinition } from "@/api/metric-definitions-client";

export interface CatalogueHealth {
  serving: number;
  broken: number;
  unverified: number;
  awaitingData: number;
  custom: number;
  disabled: number;
}

/**
 * A custom metric runs its SQL at query time and the validator stamps schema
 * status and observation date from materialized relations only, so both stay
 * absent however much data it serves. Counting those as unverified or as
 * awaiting data reports a working metric as a broken one.
 */
export function catalogueHealth(
  definitions: readonly MetricDefinition[],
): CatalogueHealth {
  const counts: CatalogueHealth = {
    serving: 0,
    broken: 0,
    unverified: 0,
    awaitingData: 0,
    custom: 0,
    disabled: 0,
  };

  for (const d of definitions) {
    if (!d.is_enabled) {
      counts.disabled += 1;
      continue;
    }
    if (d.origin === "custom") {
      counts.custom += 1;
      continue;
    }
    if (d.schema_status === "error") {
      counts.broken += 1;
      continue;
    }
    if (d.last_observed_date == null) {
      counts.awaitingData += 1;
      continue;
    }
    if (d.schema_status === "unchecked") {
      counts.unverified += 1;
      continue;
    }
    counts.serving += 1;
  }

  return counts;
}
