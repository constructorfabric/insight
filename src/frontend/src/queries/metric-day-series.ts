import { useQuery } from "@tanstack/react-query";

import { AnalyticsApiError } from "@/api/analytics-client";
import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";
import {
  queryMetricResults,
  type MetricResultsEntity,
  type MetricResultsResponse,
} from "@/api/metric-results-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import type { DayReading } from "@/lib/insight/metric-grain";

/** A view the server could not compute. Deterministic — retrying re-reads it. */
class MetricViewFailed extends Error {
  constructor(code: string) {
    super(`metric day series: ${code}`);
    this.name = "MetricViewFailed";
  }
}

function resultsEntity(
  selection: MetricEvidenceSelection
): MetricResultsEntity | null {
  switch (selection.entity.type) {
    case "person":
      return { type: "person", ids: [selection.entity.id] };
    case "persons":
      return { type: "person", ids: selection.entity.ids };
    case "tenant":
      return { type: "tenant" };
  }
}

/**
 * One reading per day the metric has one, over the whole period.
 *
 * Read from the timeseries view rather than from evidence rows: evidence is
 * paged, and a page covers the days it happens to reach rather than the period
 * asked for — while the strip drawn from it states how many days hold no
 * reading. A partial answer is a wrong answer there, not a shorter one.
 *
 * INVARIANT: a ratio's day value is its own two sides, divided here — NOT the
 * `value` the wire carries, which the server has already multiplied by the
 * metric's scale. The strip scales again when it renders, so taking the wire
 * value would multiply a percent twice.
 *
 * INVARIANT: values are summed across series, which is the identity for the
 * one-entity selection every caller passes. A selection naming several people
 * would need the metric's own computation to combine them — a ratio cannot be
 * added up — so widen this before widening the caller.
 */
function readings(
  response: MetricResultsResponse,
  metricKey: string
): DayReading[] {
  const metric = response.metrics.find((m) => m.metric_key === metricKey);
  // A view whose computation failed arrives inside a 200, in the slot the
  // working view would have taken. Reading past it would draw an empty strip
  // under "Nothing recorded in this period" — a claim about the period, from a
  // read that never reached it. Throwing puts the section on its retry.
  const failed = metric?.views.find((v) => v.view === "error");
  if (failed?.view === "error") {
    throw new MetricViewFailed(failed.code);
  }
  const view = metric?.views.find((v) => v.view === "timeseries");
  if (view?.view !== "timeseries") return [];

  const byDate = new Map<string, DayReading>();
  for (const series of view.series) {
    for (const point of series.points) {
      // A ratio reports no value on a day its denominator is zero, and the
      // sides still say what that day was measured against. Dropping the day
      // on the value alone would take the denominator with it.
      if (
        point.value == null &&
        point.numerator == null &&
        point.denominator == null
      ) {
        continue;
      }
      // Either side present makes this a ratio's day, and then BOTH sides are
      // numbers: a day with a denominator and no numerator contributed zero,
      // and reporting that side as absent would drop "of N" from the readout
      // on exactly the days a reader interrogates.
      const isRatio = point.numerator != null || point.denominator != null;
      const day = byDate.get(point.bucket_start) ?? {
        date: point.bucket_start,
        value: 0,
        numerator: isRatio ? 0 : null,
        denominator: isRatio ? 0 : null,
      };
      day.value += point.value ?? 0;
      if (isRatio) {
        day.numerator = (day.numerator ?? 0) + (point.numerator ?? 0);
        day.denominator = (day.denominator ?? 0) + (point.denominator ?? 0);
      }
      byDate.set(point.bucket_start, day);
    }
  }

  return [...byDate.values()]
    .map((day) =>
      day.denominator != null && day.denominator > 0
        ? { ...day, value: (day.numerator ?? 0) / day.denominator }
        : day
    )
    .sort((a, b) => a.date.localeCompare(b.date));
}

export function useMetricDaySeries(
  selection: MetricEvidenceSelection | null | undefined,
  enabled = true
) {
  const { session } = useAuth();
  const sessionScope = sessionAuthorizationScope(session);
  const entity = selection ? resultsEntity(selection) : null;

  return useQuery({
    queryKey: ["metric-day-series", sessionScope, selection],
    queryFn: async ({ signal }) => {
      if (!selection || !entity) {
        throw new Error("Metric day series selection is missing");
      }
      const response = await queryMetricResults(
        {
          entity,
          period: selection.period,
          metrics: [
            {
              metric_key: selection.metric_key,
              filters: selection.filters,
              views: [{ view: "timeseries", bucket: "day" }],
            },
          ],
        },
        signal
      );
      return readings(response, selection.metric_key);
    },
    enabled:
      enabled && sessionScope != null && selection != null && entity != null,
    retry: (failureCount, error) =>
      failureCount < 1 &&
      !(error instanceof MetricViewFailed) &&
      (!(error instanceof AnalyticsApiError) || error.status >= 500),
  });
}
