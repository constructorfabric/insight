import { AnalyticsApiError } from "@/api/analytics-client";
import {
  queryMetricResults,
  type MetricResult,
} from "@/api/metric-results-client";
import {
  ASSUMED_GROUPS_PER_ENTITY,
  planRequests,
  type RequestBatch,
} from "@/lib/reports/batching";
import { requestBucket, type ReportGranularity } from "@/lib/reports/rollup";
import { rowDimension, type ReportRows } from "@/lib/reports/rows";

export interface ReportRun {
  metricKeys: readonly string[];
  entityIds: readonly string[];
  range: { from: string; to: string };
  granularity: ReportGranularity;
  bucketCount: number;
  /** Defaults to the person-per-bucket shape the report started as. */
  rows?: ReportRows;
  onProgress?: (done: number, total: number) => void;
  signal?: AbortSignal;
}

/**
 * Every selected metric, over every person in scope.
 *
 * Asked at the bucket the reader chose where the server has one, and at months
 * for a quarter or a year, which it does not.
 *
 * Resolves only when every batch has arrived. A run that throws — including
 * one that was cancelled — yields nothing at all, because a file missing a few
 * batches is indistinguishable from a complete one once it is open.
 */
export async function runReport(
  run: ReportRun,
): Promise<Map<string, MetricResult>> {
  const dimension = rowDimension(run.rows ?? "people");
  const batches = planRequests(
    run.metricKeys,
    run.entityIds,
    run.bucketCount,
    dimension ? ASSUMED_GROUPS_PER_ENTITY : 1,
  );
  const merged = new Map<string, MetricResult>();
  let done = 0;
  run.onProgress?.(0, batches.length);

  for (const batch of batches) {
    await askAndMerge(batch, run, dimension, merged);
    // One tick per PLANNED batch, however many requests it took: a bar whose
    // denominator grew mid-run would jump backwards.
    run.onProgress?.((done += 1), batches.length);
  }
  return merged;
}

/**
 * One batch's answer, merged in — splitting the batch and asking again when
 * the server says the result would be too large.
 *
 * A grouped request's size is data, not arithmetic: how many repositories a
 * person touched is unknown until asked, so `ASSUMED_GROUPS_PER_ENTITY` only
 * decides how many round trips the ordinary case takes. The bound that
 * actually holds is the server's own row limit, and it is enforceable because
 * it is reported: halving the people in the batch halves the rows, and one
 * person is the floor — a single person over the limit cannot be split
 * further, so that error is the caller's to see.
 */
async function askAndMerge(
  batch: RequestBatch,
  run: ReportRun,
  dimension: string | null,
  merged: Map<string, MetricResult>,
): Promise<void> {
  try {
    const response = await queryMetricResults(
      {
        entity: { type: "person", ids: [...batch.entityIds] },
        period: run.range,
        metrics: batch.metricKeys.map((metric_key) => ({
          metric_key,
          views: [
            {
              view: "timeseries",
              bucket: requestBucket(run.granularity),
              // No group limit: repositories are few enough to name them all,
              // and a capped request would fold the tail into a remainder row
              // that a per-repository table cannot label.
              ...(dimension ? { dimensions: [dimension] } : {}),
            },
          ],
        })),
      },
      run.signal,
    );
    for (const result of response.metrics) {
      mergeSeries(merged, result);
    }
  } catch (error) {
    if (!isResultTooLarge(error) || batch.entityIds.length < 2) throw error;
    const half = Math.ceil(batch.entityIds.length / 2);
    await askAndMerge(
      { ...batch, entityIds: batch.entityIds.slice(0, half) },
      run,
      dimension,
      merged,
    );
    await askAndMerge(
      { ...batch, entityIds: batch.entityIds.slice(half) },
      run,
      dimension,
      merged,
    );
  }
}

/**
 * Whether the service refused a request because its ANSWER would be too big,
 * as opposed to any other bad request.
 *
 * Narrow on purpose: retrying a malformed request by halving it would turn one
 * clear failure into a storm of identical ones.
 */
export function isResultTooLarge(error: unknown): boolean {
  if (!(error instanceof AnalyticsApiError) || error.status !== 400) return false;
  const violations = (
    error.body as
      | { context?: { field_violations?: Array<{ reason?: string }> } }
      | null
      | undefined
  )?.context?.field_violations;
  return (
    Array.isArray(violations) &&
    violations.some((violation) => violation.reason === "metric_result_too_large")
  );
}

/**
 * People arrive across batches, so a metric's series accumulate rather than
 * replace: the second batch for a metric carries different people, not a
 * newer answer about the same ones.
 */
function mergeSeries(
  into: Map<string, MetricResult>,
  result: MetricResult,
): void {
  const held = into.get(result.metric_key);
  if (!held) {
    into.set(result.metric_key, result);
    return;
  }
  const heldView = held.views.find((view) => view.view === "timeseries");
  const incoming = result.views.find((view) => view.view === "timeseries");
  if (heldView?.view === "timeseries" && incoming?.view === "timeseries") {
    heldView.series = [...heldView.series, ...incoming.series];
  }
}
