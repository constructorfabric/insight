/**
 * The rows behind one metric, fetched for showing inline.
 *
 * What matters here is when it does NOT fetch. A section renders one of these
 * per headline metric, so a hook that asks anyway — with no session, with no
 * selection, or for a metric that offers no detail — turns one screen into
 * several pointless round trips.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as drilldown from "@/api/metric-drilldown-client";
import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";

vi.mock("@/api/metric-drilldown-client");

const session = vi.hoisted(() => ({ value: { scope: "tenant-a" } as unknown }));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: session.value }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (s: unknown) =>
    s == null ? null : (s as { scope: string }).scope,
}));

import { DETAIL_LIMIT, useMetricDetail } from "./metric-detail";

const SELECTION: MetricEvidenceSelection = {
  metric_key: "git.commits",
  entity: { type: "person", id: "019e27bc-dec0-7626-81a9-c5524662a6a9" },
  period: { from: "2026-03-01", to: "2026-03-31" },
  filters: [],
  display_dimensions: [],
};

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper };
}

beforeEach(() => {
  vi.resetAllMocks();
  session.value = { scope: "tenant-a" };
});

describe("useMetricDetail", () => {
  it("asks for one page, bounded", async () => {
    // A section is a first look, not an export: it takes what one request
    // returns rather than paging until the source runs out.
    vi.mocked(drilldown.queryMetricDrilldown).mockResolvedValue({
      selection: SELECTION,
      columns: [{ key: "date", label: "Date", type: "date" }],
      rows: [{ values: { date: "2026-03-01" } }],
      next_cursor: "there-is-more",
    });
    const { wrapper } = harness();
    const { result } = renderHook(() => useMetricDetail(SELECTION), {
      wrapper,
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(drilldown.queryMetricDrilldown).toHaveBeenCalledTimes(1);
    expect(
      vi.mocked(drilldown.queryMetricDrilldown).mock.calls[0][0]
    ).toMatchObject({
      metric_key: "git.commits",
      limit: DETAIL_LIMIT,
    });
    expect(result.current.data?.rows).toHaveLength(1);
  });

  it("does not fetch for a metric that offers no detail", async () => {
    // The caller passes `enabled: false` for a metric declaring no grain.
    // Asking anyway would spend a request to be told there is nothing.
    const { wrapper } = harness();
    renderHook(() => useMetricDetail(SELECTION, false), { wrapper });
    await new Promise((r) => setTimeout(r, 0));
    expect(drilldown.queryMetricDrilldown).not.toHaveBeenCalled();
  });

  it("does not fetch without a selection", async () => {
    const { wrapper } = harness();
    renderHook(() => useMetricDetail(null), { wrapper });
    await new Promise((r) => setTimeout(r, 0));
    expect(drilldown.queryMetricDrilldown).not.toHaveBeenCalled();
  });

  it("does not fetch before a session exists", async () => {
    // On a cold load the session resolves after the first render. Firing then
    // produces a 401 the screen would have to explain, for a request that was
    // never going to work.
    session.value = null;
    const { wrapper } = harness();
    renderHook(() => useMetricDetail(SELECTION), { wrapper });
    await new Promise((r) => setTimeout(r, 0));
    expect(drilldown.queryMetricDrilldown).not.toHaveBeenCalled();
  });

  it("keeps two different selections apart", async () => {
    // The key carries the selection, so a second metric does not read the
    // first one's rows out of the cache.
    vi.mocked(drilldown.queryMetricDrilldown).mockResolvedValue({
      selection: SELECTION,
      columns: [],
      rows: [],
      next_cursor: null,
    });
    const { wrapper } = harness();
    const { result, rerender } = renderHook(
      ({ s }: { s: MetricEvidenceSelection }) => useMetricDetail(s),
      { wrapper, initialProps: { s: SELECTION } }
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    rerender({ s: { ...SELECTION, metric_key: "git.prs_merged" } });
    await waitFor(() =>
      expect(drilldown.queryMetricDrilldown).toHaveBeenCalledTimes(2)
    );
  });
});
