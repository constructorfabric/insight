import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import * as metricsClient from "@/api/metrics-client";
import * as resultsClient from "@/api/metric-results-client";

import { MetricDetail } from "./metric-detail";

vi.mock("@/api/metrics-client");
vi.mock("@/api/metric-results-client");

const METRIC: metricsClient.CustomMetric = {
  metric_key: "example.lines",
  label: "Accepted lines",
  short_label: null,
  description: "Synthetic sample metric.",
  explanation: null,
  entity_type: "person",
  unit: "lines",
  format: "integer",
  direction: "higher_is_better",
  computation: "sum",
  scale: null,
  peer_cohort_key: null,
  transform: null,
  source_key: "example_source",
  observation_sql: "SELECT tenant_id, value FROM example_source",
  measures: ["lines"],
  dimensions: ["repo"],
  inputs: [{ role: "value", measure_key: "lines" }],
  origin: "custom",
};

function renderDetail(over: Partial<Parameters<typeof MetricDetail>[0]> = {}) {
  const onEdit = vi.fn();
  const onDelete = vi.fn();
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  render(
    <MetricDetail
      metricKey="example.lines"
      onEdit={onEdit}
      onDelete={onDelete}
      {...over}
    />,
    { wrapper }
  );
  return { onEdit, onDelete };
}

const ENTITY_ID = "00000000-0000-0000-0000-0000000000aa";

beforeEach(() => vi.resetAllMocks());

describe("MetricDetail", () => {
  it("shows a load error when the metric fails to fetch", async () => {
    vi.mocked(metricsClient.getCustomMetric).mockRejectedValue(
      new Error("request failed")
    );
    renderDetail();
    expect(
      await screen.findByText("Failed to load this metric")
    ).toBeInTheDocument();
  });

  it("renders the metric header, key, and observation SQL", async () => {
    vi.mocked(metricsClient.getCustomMetric).mockResolvedValue(METRIC);
    renderDetail();
    expect(await screen.findByText("Accepted lines")).toBeInTheDocument();
    expect(screen.getByText("example.lines")).toBeInTheDocument();
    expect(
      screen.getByText("SELECT tenant_id, value FROM example_source")
    ).toBeInTheDocument();
  });

  it("keeps Preview disabled until entity ids and a period are set", async () => {
    vi.mocked(metricsClient.getCustomMetric).mockResolvedValue(METRIC);
    renderDetail();
    await screen.findByText("Accepted lines");

    const preview = screen.getByRole("button", { name: /preview/i });
    expect(preview).toBeDisabled();

    await userEvent.type(screen.getByLabelText("Entity ids"), ENTITY_ID);
    await userEvent.type(screen.getByLabelText("From"), "2026-01-01");
    await userEvent.type(screen.getByLabelText("To"), "2026-01-31");
    expect(preview).toBeEnabled();
  });

  it("previews the metric and renders the result table", async () => {
    vi.mocked(metricsClient.getCustomMetric).mockResolvedValue(METRIC);
    vi.mocked(resultsClient.queryMetricResults).mockResolvedValue({
      metrics: [
        {
          metric_key: "example.lines",
          label: "Accepted lines",
          unit: "lines",
          format: "integer",
          direction: "higher_is_better",
          computation: "sum",
          views: [
            { view: "period", values: [{ entity_id: ENTITY_ID, value: 7 }] },
          ],
        },
      ],
    });
    renderDetail();
    await screen.findByText("Accepted lines");

    await userEvent.type(screen.getByLabelText("Entity ids"), ENTITY_ID);
    await userEvent.type(screen.getByLabelText("From"), "2026-01-01");
    await userEvent.type(screen.getByLabelText("To"), "2026-01-31");
    await userEvent.click(screen.getByRole("button", { name: /preview/i }));

    expect(await screen.findByText("7")).toBeInTheDocument();
    expect(resultsClient.queryMetricResults).toHaveBeenCalledWith({
      entity: { type: "person", ids: [ENTITY_ID] },
      period: { from: "2026-01-01", to: "2026-01-31" },
      metrics: [{ metric_key: "example.lines", views: [{ view: "period" }] }],
    });
  });

  it("surfaces a preview failure", async () => {
    vi.mocked(metricsClient.getCustomMetric).mockResolvedValue(METRIC);
    vi.mocked(resultsClient.queryMetricResults).mockRejectedValue(
      new Error("nope")
    );
    renderDetail();
    await screen.findByText("Accepted lines");

    await userEvent.type(screen.getByLabelText("Entity ids"), ENTITY_ID);
    await userEvent.type(screen.getByLabelText("From"), "2026-01-01");
    await userEvent.type(screen.getByLabelText("To"), "2026-01-31");
    await userEvent.click(screen.getByRole("button", { name: /preview/i }));

    expect(await screen.findByText("Preview failed")).toBeInTheDocument();
  });

  it("fires edit and delete callbacks", async () => {
    vi.mocked(metricsClient.getCustomMetric).mockResolvedValue(METRIC);
    const { onEdit, onDelete } = renderDetail();
    await screen.findByText("Accepted lines");

    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(onEdit).toHaveBeenCalledWith("example.lines");

    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onDelete).toHaveBeenCalledWith("example.lines");
  });
});
