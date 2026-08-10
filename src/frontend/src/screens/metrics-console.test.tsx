import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { CustomMetric, CustomMetricSummary } from "@/api/metrics-client";

let listState: {
  data?: CustomMetricSummary[];
  isPending: boolean;
  isError: boolean;
};
let metricState: { data?: CustomMetric; isError: boolean };
let deleteState: { isError: boolean };

// Mutations invoke their success callback synchronously so the screen's
// onSuccess wiring (select/close/reset) is exercised.
const createMutate = vi.fn(
  (_body: unknown, opts?: { onSuccess?: (m: { metric_key: string }) => void }) =>
    opts?.onSuccess?.({ metric_key: "new.metric" })
);
const updateMutate = vi.fn(
  (_body: unknown, opts?: { onSuccess?: () => void }) => opts?.onSuccess?.()
);
const deleteMutate = vi.fn(
  (_key: string, opts?: { onSuccess?: () => void }) => opts?.onSuccess?.()
);

vi.mock("@/queries/custom-metrics", () => ({
  useCustomMetrics: () => listState,
  useCreateCustomMetric: () => ({
    mutate: createMutate,
    isPending: false,
    error: null,
  }),
  useDeleteCustomMetric: () => ({ mutate: deleteMutate, ...deleteState }),
  useUpdateCustomMetric: () => ({
    mutate: updateMutate,
    isPending: false,
    error: null,
  }),
  useCustomMetric: () => metricState,
}));

vi.mock("@/components/ui/sidebar", () => ({ SidebarTrigger: () => null }));

// Stubs that actually invoke their callback props, so the screen's delete /
// edit-open / create-success / update branches run in tests.
vi.mock("@/components/widgets/metrics-console/metric-detail", () => ({
  MetricDetail: ({
    metricKey,
    onEdit,
    onDelete,
  }: {
    metricKey: string;
    onEdit: (metricKey: string) => void;
    onDelete: (metricKey: string) => void;
  }) => (
    <div>
      detail:{metricKey}
      <button onClick={() => onEdit(metricKey)}>stub-edit</button>
      <button onClick={() => onDelete(metricKey)}>stub-delete</button>
    </div>
  ),
}));

vi.mock("@/components/widgets/metrics-console/metric-editor-dialog", () => ({
  MetricEditorDialog: ({
    mode,
    onSubmit,
    onOpenChange,
  }: {
    mode: string;
    onSubmit: (d: { metric_key: string }) => void;
    onOpenChange: (open: boolean) => void;
  }) => (
    <div>
      editor:{mode}
      <button onClick={() => onSubmit({ metric_key: "draft.metric" })}>
        stub-submit
      </button>
      <button onClick={() => onOpenChange(false)}>stub-close</button>
    </div>
  ),
}));

// The screen turns the editor draft into a graph before mutating; the stub
// hands back a partial draft, so stub the converter to a passthrough.
vi.mock("@/lib/metrics-console/metric-graph", async (importOriginal) => {
  const actual =
    await importOriginal<
      typeof import("@/lib/metrics-console/metric-graph")
    >();
  return { ...actual, draftToGraph: (d: unknown) => d, graphToDraft: (g: unknown) => g };
});

import { MetricsConsoleScreen } from "./metrics-console";

function summary(over: Partial<CustomMetricSummary> = {}): CustomMetricSummary {
  return {
    metric_key: "example.accepted_lines",
    label: "Accepted lines",
    computation: "sum",
    entity_type: "person",
    ...over,
  };
}

const METRIC: CustomMetric = {
  metric_key: "example.accepted_lines",
  label: "Accepted lines",
  short_label: null,
  description: null,
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
  observation_sql: "SELECT 1",
  measures: ["accepted_lines"],
  dimensions: ["repo"],
  inputs: [{ role: "value", measure_key: "accepted_lines" }],
  origin: "custom",
};

beforeEach(() => {
  listState = { data: [summary()], isPending: false, isError: false };
  metricState = { data: METRIC, isError: false };
  deleteState = { isError: false };
  createMutate.mockClear();
  updateMutate.mockClear();
  deleteMutate.mockClear();
});

describe("MetricsConsoleScreen", () => {
  it("shows a spinner while the list is pending", () => {
    listState = { data: undefined, isPending: true, isError: false };
    render(<MetricsConsoleScreen />);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows an error alert when the list fails", () => {
    listState = { data: undefined, isPending: false, isError: true };
    render(<MetricsConsoleScreen />);
    expect(
      screen.getByText("Failed to load custom metrics")
    ).toBeInTheDocument();
  });

  it("shows an empty-list hint and no selection when there are no metrics", () => {
    listState = { data: [], isPending: false, isError: false };
    render(<MetricsConsoleScreen />);
    expect(
      screen.getByText("No custom metrics yet. Create one to get started.")
    ).toBeInTheDocument();
    expect(screen.getByText("No metric selected")).toBeInTheDocument();
  });

  it("selects a metric and renders its detail pane", async () => {
    render(<MetricsConsoleScreen />);
    await userEvent.click(screen.getByText("Accepted lines"));
    expect(
      screen.getByText("detail:example.accepted_lines")
    ).toBeInTheDocument();
  });

  it("creates a metric, then selects it and closes the dialog", async () => {
    render(<MetricsConsoleScreen />);
    await userEvent.click(screen.getByRole("button", { name: /new metric/i }));
    expect(screen.getByText("editor:create")).toBeInTheDocument();

    await userEvent.click(screen.getByText("stub-submit"));
    expect(createMutate).toHaveBeenCalledWith(
      { metric_key: "draft.metric" },
      expect.any(Object)
    );
    expect(screen.getByText("detail:new.metric")).toBeInTheDocument();
    expect(screen.queryByText("editor:create")).not.toBeInTheDocument();
  });

  it("deletes the selected metric and clears the selection", async () => {
    render(<MetricsConsoleScreen />);
    await userEvent.click(screen.getByText("Accepted lines"));
    await userEvent.click(screen.getByText("stub-delete"));
    expect(deleteMutate).toHaveBeenCalledWith(
      "example.accepted_lines",
      expect.any(Object)
    );
    expect(screen.getByText("No metric selected")).toBeInTheDocument();
  });

  it("opens the edit dialog and submits an update", async () => {
    render(<MetricsConsoleScreen />);
    await userEvent.click(screen.getByText("Accepted lines"));
    await userEvent.click(screen.getByText("stub-edit"));
    expect(screen.getByText("editor:edit")).toBeInTheDocument();

    await userEvent.click(screen.getByText("stub-submit"));
    expect(updateMutate).toHaveBeenCalledWith(
      { metric_key: "draft.metric" },
      expect.any(Object)
    );
  });

  it("shows a spinner in the edit dialog while the metric loads", async () => {
    metricState = { data: undefined, isError: false };
    render(<MetricsConsoleScreen />);
    await userEvent.click(screen.getByText("Accepted lines"));
    await userEvent.click(screen.getByText("stub-edit"));
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows an error in the edit dialog when the metric fails to load", async () => {
    metricState = { data: undefined, isError: true };
    render(<MetricsConsoleScreen />);
    await userEvent.click(screen.getByText("Accepted lines"));
    await userEvent.click(screen.getByText("stub-edit"));
    expect(screen.getByText("Failed to load this metric")).toBeInTheDocument();
  });

  it("surfaces a delete failure", () => {
    deleteState = { isError: true };
    render(<MetricsConsoleScreen />);
    expect(
      screen.getByText("Could not delete the metric. Try again.")
    ).toBeInTheDocument();
  });
});
