/** The server rejects a request carrying more than this many metrics. */
export const MAX_METRICS_PER_REQUEST = 50;

/**
 * How many values one request may carry back.
 *
 * NOT the server's row limit, which is checked per view and therefore never
 * sees the metric count: fifty metrics for several hundred people across a
 * year of months satisfies every check it makes and still returns a response
 * of a size nothing bounded. Budgeting the product is what keeps each request
 * comparable in weight — which is also what makes a progress indicator honest.
 */
export const MAX_VALUES_PER_REQUEST = 4500;

export interface RequestBatch {
  metricKeys: string[];
  entityIds: string[];
}

function chunk<T>(items: readonly T[], size: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += size) out.push(items.slice(i, i + size));
  return out;
}

/**
 * Split a selection into requests that each stay within both caps.
 *
 * Metrics are capped first because their limit is hard; the people per
 * request then follow from what is left of the value budget.
 */
export function planRequests(
  metricKeys: readonly string[],
  entityIds: readonly string[],
  bucketCount: number,
): RequestBatch[] {
  if (metricKeys.length === 0 || entityIds.length === 0) return [];
  const metricBatches = chunk([...metricKeys], MAX_METRICS_PER_REQUEST);
  return metricBatches.flatMap((keys) => {
    const perEntity = Math.max(1, keys.length * (bucketCount + 1));
    const entitiesPerRequest = Math.max(
      1,
      Math.floor(MAX_VALUES_PER_REQUEST / perEntity),
    );
    return chunk([...entityIds], entitiesPerRequest).map((ids) => ({
      metricKeys: keys,
      entityIds: ids,
    }));
  });
}
