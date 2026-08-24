import {
  forEntity,
  type EntityMetricData,
  type NormalizedMetricResult,
} from "@/lib/metrics/collection";

/** The single tenant row of each view — entity_id is opaque here (the backend
 *  stamps the organization id), so "the only entity" IS the selection. */
export function tenantData(r: NormalizedMetricResult): EntityMetricData {
  const id = firstEntityId(r);
  return forEntity(r, id ?? "");
}

function firstEntityId(r: NormalizedMetricResult): string | null {
  return (
    r.period?.values[0]?.entity_id ??
    r.timeseries?.series[0]?.entity_id ??
    r.breakdown?.values[0]?.entity_id ??
    r.histogram?.values[0]?.entity_id ??
    null
  );
}
