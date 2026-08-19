import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ query: vi.fn() }));
vi.mock("@/api/metric-results-client", () => ({ queryMetricResults: mocks.query }));

import { probeComputations } from "@/queries/report-catalogue";

beforeEach(() => {
  mocks.query.mockReset();
  mocks.query.mockImplementation(
    (body: { metrics: Array<{ metric_key: string }> }) =>
      Promise.resolve({
        metrics: body.metrics.map((m) => ({
          metric_key: m.metric_key,
          computation: m.metric_key.startsWith("ratio") ? "ratio" : "sum",
        })),
      }),
  );
});

describe("probeComputations", () => {
  it("splits the catalogue to respect the per-request metric cap", async () => {
    const keys = Array.from({ length: 130 }, (_, i) => `m${i}`);
    const map = await probeComputations(keys, "p1", "2026-05-01");

    expect(map.size).toBe(130);
    expect(mocks.query.mock.calls.length).toBe(3);
    for (const [body] of mocks.query.mock.calls) {
      expect(body.metrics.length).toBeLessThanOrEqual(50);
    }
  });

  it("asks about one person over one day — it wants the shape, not the data", () => {
    return probeComputations(["a"], "p1", "2026-05-01").then(() => {
      const [body] = mocks.query.mock.calls[0] ?? [];
      expect(body.entity).toEqual({ type: "person", ids: ["p1"] });
      expect(body.period).toEqual({ from: "2026-05-01", to: "2026-05-01" });
      expect(body.metrics[0].views).toEqual([{ view: "period" }]);
    });
  });

  it("reports each metric's computation as the server gave it", async () => {
    const map = await probeComputations(["sum_a", "ratio_b"], "p1", "2026-05-01");
    expect(map.get("sum_a")).toBe("sum");
    expect(map.get("ratio_b")).toBe("ratio");
  });

  it("asks nothing when the catalogue is empty", async () => {
    expect((await probeComputations([], "p1", "2026-05-01")).size).toBe(0);
    expect(mocks.query).not.toHaveBeenCalled();
  });
});
