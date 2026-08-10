import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import "@/i18n";

import { AnalyticsApiError } from "@/api/analytics-client";
import { graphToDraft } from "@/lib/metrics-console/metric-graph";

import { MetricEditorDialog } from "./metric-editor-dialog";

function setup(
  overrides: Partial<Parameters<typeof MetricEditorDialog>[0]> = {}
) {
  const onSubmit = vi.fn();
  const onOpenChange = vi.fn();
  render(
    <MetricEditorDialog
      open
      onOpenChange={onOpenChange}
      mode="create"
      onSubmit={onSubmit}
      isPending={false}
      error={null}
      {...overrides}
    />
  );
  return { onSubmit, onOpenChange };
}

describe("MetricEditorDialog", () => {
  it("create mode keeps Save disabled until the required fields are set", async () => {
    const { onSubmit } = setup();
    expect(screen.getByText("New custom metric")).toBeInTheDocument();

    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    await userEvent.type(screen.getByLabelText("Metric key"), "example.lines");
    await userEvent.type(screen.getByLabelText("Label"), "Lines");
    await userEvent.type(screen.getByLabelText("Source key"), "example_source");
    await userEvent.type(screen.getByLabelText("Observation SQL"), "SELECT 1");
    await userEvent.type(screen.getByLabelText("Measures"), "lines");
    await userEvent.type(screen.getByLabelText("Value measure"), "lines");
    expect(save).toBeEnabled();

    await userEvent.click(save);
    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      metric_key: "example.lines",
      label: "Lines",
      source_key: "example_source",
      value_measure: "lines",
    });
  });

  it("ratio mode reveals numerator, denominator, and scale fields", async () => {
    setup();
    await userEvent.selectOptions(
      screen.getByLabelText("Computation"),
      "ratio"
    );
    expect(screen.getByLabelText("Numerator measure")).toBeInTheDocument();
    expect(screen.getByLabelText("Denominator measure")).toBeInTheDocument();
    expect(screen.getByLabelText("Scale")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Value measure")
    ).not.toBeInTheDocument();
  });

  it("edits every field and submits a complete ratio draft", async () => {
    const { onSubmit } = setup();

    await userEvent.type(screen.getByLabelText("Metric key"), "example.rate");
    await userEvent.type(screen.getByLabelText("Label"), "Rate");
    await userEvent.type(screen.getByLabelText("Short label"), "R");
    await userEvent.clear(screen.getByLabelText("Entity type"));
    await userEvent.type(screen.getByLabelText("Entity type"), "person");
    await userEvent.type(screen.getByLabelText("Description"), "desc");
    await userEvent.type(screen.getByLabelText("Explanation"), "why");
    await userEvent.selectOptions(screen.getByLabelText("Format"), "percent");
    await userEvent.selectOptions(
      screen.getByLabelText("Direction"),
      "higher_is_better"
    );
    await userEvent.selectOptions(
      screen.getByLabelText("Computation"),
      "ratio"
    );
    await userEvent.type(screen.getByLabelText("Unit"), "%");
    await userEvent.type(screen.getByLabelText("Scale"), "100");
    await userEvent.type(
      screen.getByLabelText("Peer cohort key"),
      "team"
    );
    await userEvent.type(
      screen.getByLabelText("Source key"),
      "example_source"
    );
    await userEvent.type(
      screen.getByLabelText("Observation SQL"),
      "SELECT 1"
    );
    await userEvent.type(screen.getByLabelText("Measures"), "num, den");
    await userEvent.type(screen.getByLabelText("Dimensions"), "repo");
    await userEvent.type(screen.getByLabelText("Numerator measure"), "num");
    await userEvent.type(screen.getByLabelText("Denominator measure"), "den");
    await userEvent.type(screen.getByLabelText("Multiplier"), "2");
    await userEvent.type(screen.getByLabelText("Offset"), "1");
    await userEvent.type(screen.getByLabelText("Clamp min"), "0");
    await userEvent.type(screen.getByLabelText("Clamp max"), "5");

    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeEnabled();
    await userEvent.click(save);

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      metric_key: "example.rate",
      label: "Rate",
      short_label: "R",
      description: "desc",
      explanation: "why",
      format: "percent",
      direction: "higher_is_better",
      computation: "ratio",
      unit: "%",
      scale: "100",
      peer_cohort_key: "team",
      source_key: "example_source",
      measures: "num, den",
      dimensions: "repo",
      numerator_measure: "num",
      denominator_measure: "den",
      transform_multiplier: "2",
      transform_offset: "1",
      transform_clamp_min: "0",
      transform_clamp_max: "5",
    });
  });

  it("edit mode prefills from an existing graph and locks the key", () => {
    setup({
      mode: "edit",
      initial: graphToDraft({
        metric_key: "example.lines",
        label: "Lines",
        short_label: null,
        description: "desc",
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
        observation_sql: "SELECT 2",
        measures: ["lines"],
        dimensions: ["repo"],
        inputs: [{ role: "value", measure_key: "lines" }],
      }),
    });
    expect(screen.getByText("Edit custom metric")).toBeInTheDocument();
    expect(screen.getByLabelText("Metric key")).toHaveValue("example.lines");
    expect(screen.getByLabelText("Metric key")).toBeDisabled();
    expect(screen.getByLabelText("Observation SQL")).toHaveValue("SELECT 2");
    expect(screen.getByLabelText("Description")).toHaveValue("desc");
  });

  it("surfaces the API field-violation reason", () => {
    setup({
      error: new AnalyticsApiError(400, {
        context: {
          field_violations: [
            { field: "observation_sql", description: "bad sql" },
          ],
        },
      }),
    });
    expect(screen.getByText("bad sql")).toBeInTheDocument();
  });

  it("Cancel requests close without submitting", async () => {
    const { onOpenChange, onSubmit } = setup();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
