import {
  queryMetricResults,
  type MetricResult,
} from "@/api/metric-results-client";
import { planRequests } from "@/lib/reports/batching";
import { requestBucket, type ReportGranularity } from "@/lib/reports/rollup";

export interface ReportRun {
  metricKeys: readonly string[];
  entityIds: readonly string[];
  range: { from: string; to: string };
  granularity: ReportGranularity;
  bucketCount: number;
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
  const batches = planRequests(run.metricKeys, run.entityIds, run.bucketCount);
  const merged = new Map<string, MetricResult>();
  let done = 0;
  run.onProgress?.(0, batches.length);

  for (const batch of batches) {
    const response = await queryMetricResults(
      {
        entity: { type: "person", ids: [...batch.entityIds] },
        period: run.range,
        metrics: batch.metricKeys.map((metric_key) => ({
          metric_key,
          views: [{ view: "timeseries", bucket: requestBucket(run.granularity) }],
        })),
      },
      run.signal,
    );
    for (const result of response.metrics) {
      mergeSeries(merged, result);
    }
    run.onProgress?.((done += 1), batches.length);
  }
  return merged;
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
