import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ query: vi.fn() }));
vi.mock("@/api/metric-results-client", () => ({
  queryMetricResults: mocks.query,
}));

import { AnalyticsApiError } from "@/api/analytics-client";
import { isResultTooLarge, runReport } from "@/lib/reports/run-report";
import { buildMetricErrorView } from "@/mocks/metric-results-factory";

const tooLarge = () =>
  new AnalyticsApiError(400, {
    context: {
      field_violations: [
        { field: "metrics.views", reason: "metric_result_too_large" },
      ],
    },
  });

const seriesFor = (metricKey: string, entityIds: string[]) => ({
  metric_key: metricKey,
  label: metricKey,
  computation: "sum",
  views: [
    {
      view: "timeseries",
      bucket: "month",
      series: entityIds.map((entity_id) => ({
        entity_id,
        dimensions: [],
        points: [{ bucket_start: "2026-01-01", value: 1 }],
      })),
    },
  ],
});

const people = (count: number) =>
  Array.from({ length: count }, (_, i) => `p${i}`);

beforeEach(() => {
  mocks.query.mockReset();
  mocks.query.mockImplementation((body: { entity: { ids: string[] }; metrics: Array<{ metric_key: string }> }) =>
    Promise.resolve({
      metrics: body.metrics.map((m) => seriesFor(m.metric_key, body.entity.ids)),
    }),
  );
});

describe("runReport", () => {
  it("keeps every person when a metric spans several batches", () => {
    // The second batch for a metric carries different people, not a fresher
    // answer about the same ones — replacing would silently drop a chunk of
    // the roster from the file.
    return runReport({
      metricKeys: ["a"],
      entityIds: people(900),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity: "month" as const,
      bucketCount: 12,
    }).then((merged) => {
      const view = merged.get("a")?.views[0];
      expect(view?.view).toBe("timeseries");
      if (view?.view !== "timeseries") throw new Error("expected a series");
      expect(new Set(view.series.map((s) => s.entity_id)).size).toBe(900);
      expect(mocks.query.mock.calls.length).toBeGreaterThan(1);
    });
  });

  it("reports progress once per batch, ending at the total", async () => {
    const seen: Array<[number, number]> = [];
    await runReport({
      metricKeys: ["a", "b"],
      entityIds: people(900),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity: "month" as const,
      bucketCount: 12,
      onProgress: (done, total) => seen.push([done, total]),
    });
    expect(seen[0]?.[0]).toBe(0);
    const [lastDone, lastTotal] = seen.at(-1) ?? [];
    expect(lastDone).toBe(lastTotal);
    expect(seen).toHaveLength((lastTotal ?? 0) + 1);
  });

  it("carries a metric whose computation failed instead of throwing", async () => {
    // The server answers 200 with an error view in the timeseries slot; the
    // run completes and the metric simply has no series to merge.
    mocks.query.mockResolvedValueOnce({
      metrics: [
        {
          metric_key: "a",
          label: "a",
          computation: "sum",
          views: [buildMetricErrorView()],
        },
      ],
    });
    const merged = await runReport({
      metricKeys: ["a"],
      entityIds: people(10),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity: "month" as const,
      bucketCount: 12,
    });
    expect(merged.get("a")?.views[0]?.view).toBe("error");
  });

  it("yields nothing when a batch fails", async () => {
    mocks.query.mockRejectedValueOnce(new Error("network"));
    await expect(
      runReport({
        metricKeys: ["a"],
        entityIds: people(10),
        range: { from: "2026-01-01", to: "2026-12-31" },
        granularity: "month" as const,
      bucketCount: 12,
      }),
    ).rejects.toThrow("network");
  });

  it.each([
    ["day", "day"],
    ["week", "week"],
    ["month", "month"],
    // No such bucket server-side, so months are asked for and added up.
    ["quarter", "month"],
    ["year", "month"],
  ] as const)("asks for %s as bucket %s", async (granularity, bucket) => {
    await runReport({
      metricKeys: ["a"],
      entityIds: people(1),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity,
      bucketCount: 4,
    });
    expect(mocks.query.mock.calls[0]?.[0].metrics[0].views).toEqual([
      { view: "timeseries", bucket },
    ]);
  });

  it("groups by repository only when rows are repositories", async () => {
    await runReport({
      metricKeys: ["a"],
      entityIds: people(1),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity: "month",
      bucketCount: 4,
      rows: "repositories",
    });
    expect(mocks.query.mock.calls[0]?.[0].metrics[0].views).toEqual([
      { view: "timeseries", bucket: "month", dimensions: ["repository"] },
    ]);
  });

  it("budgets grouped requests for several groups per person", async () => {
    // A grouped series answers once per repository a person touched, so the
    // same roster has to be split into more requests than it would ungrouped.
    const run = {
      metricKeys: ["a"],
      entityIds: people(300),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity: "month" as const,
      bucketCount: 12,
    };
    await runReport(run);
    const ungrouped = mocks.query.mock.calls.length;
    mocks.query.mockClear();
    await runReport({ ...run, rows: "repositories" });
    expect(mocks.query.mock.calls.length).toBeGreaterThan(ungrouped);
  });

  it("halves a batch the service refused as too large, and keeps every person", async () => {
    // A grouped request's size is data: the assumed groups-per-person only
    // decides the first attempt. What must hold is that a refusal costs a
    // round trip, not the report.
    const asked: string[][] = [];
    mocks.query.mockImplementation(
      (body: { entity: { ids: string[] }; metrics: Array<{ metric_key: string }> }) => {
        asked.push(body.entity.ids);
        if (body.entity.ids.length > 1) return Promise.reject(tooLarge());
        return Promise.resolve({
          metrics: body.metrics.map((m) => seriesFor(m.metric_key, body.entity.ids)),
        });
      },
    );

    const results = await runReport({
      metricKeys: ["a"],
      entityIds: people(4),
      range: { from: "2026-01-01", to: "2026-12-31" },
      granularity: "month",
      bucketCount: 4,
      rows: "repositories",
    });

    const answered = results.get("a")?.views[0];
    const covered =
      answered?.view === "timeseries"
        ? answered.series.map((s) => s.entity_id).sort()
        : [];
    expect(covered).toEqual(["p0", "p1", "p2", "p3"]);
    // Every retry is strictly smaller than the request that was refused.
    expect(asked.some((ids) => ids.length === 1)).toBe(true);
  });

  it("gives up on a single person the service still refuses", async () => {
    // One person is the floor: there is nothing left to split, so the caller
    // sees the refusal instead of an endless retry.
    mocks.query.mockImplementation(() => Promise.reject(tooLarge()));
    await expect(
      runReport({
        metricKeys: ["a"],
        entityIds: people(1),
        range: { from: "2026-01-01", to: "2026-12-31" },
        granularity: "month",
        bucketCount: 4,
        rows: "repositories",
      }),
    ).rejects.toBeInstanceOf(AnalyticsApiError);
  });

  it("never retries a refusal that is not about size", async () => {
    // Halving a malformed request would turn one clear failure into a storm.
    mocks.query.mockImplementation(() =>
      Promise.reject(new AnalyticsApiError(400, { context: { field_violations: [{ reason: "INVALID" }] } })),
    );
    await expect(
      runReport({
        metricKeys: ["a"],
        entityIds: people(4),
        range: { from: "2026-01-01", to: "2026-12-31" },
        granularity: "month",
        bucketCount: 4,
        rows: "repositories",
      }),
    ).rejects.toBeInstanceOf(AnalyticsApiError);
    expect(mocks.query.mock.calls.length).toBe(1);
  });
});

describe("isResultTooLarge", () => {
  it("recognises the row-limit refusal and nothing else", () => {
    expect(isResultTooLarge(tooLarge())).toBe(true);
    expect(isResultTooLarge(new AnalyticsApiError(400, { context: {} }))).toBe(false);
    expect(isResultTooLarge(new AnalyticsApiError(500, null))).toBe(false);
    expect(isResultTooLarge(new Error("network"))).toBe(false);
  });
});
