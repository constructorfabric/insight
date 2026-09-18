import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { RangePicker } from "./range-picker";

describe("<RangePicker>", () => {
  it("offers only the windows the board declared, under names a reader says", () => {
    render(
      <RangePicker
        offered={["PDC", "P30D", "inf"]}
        selected="P30D"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Yesterday" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Last 30 days" })).toBeVisible();
    expect(screen.getByRole("button", { name: "All time" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Last quarter" }),
    ).not.toBeInTheDocument();
  });

  it("says which window is being read", () => {
    render(
      <RangePicker
        offered={["PDC", "P30D"]}
        selected="P30D"
        onSelect={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Last 30 days" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Yesterday" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("hands the picked token back", async () => {
    const onSelect = vi.fn();
    render(
      <RangePicker
        offered={["PDC", "P30D"]}
        selected="P30D"
        onSelect={onSelect}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: "Yesterday" }));

    expect(onSelect).toHaveBeenCalledWith("PDC");
  });

  it("shows a custom interval as its two dates, selected", () => {
    render(
      <RangePicker
        offered={["PDC", "P30D"]}
        selected="2026-08-01/2026-09-01"
        onSelect={vi.fn()}
      />,
    );

    const custom = screen.getByRole("button", { name: /2026-08-01/ });

    expect(custom).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: "Last 30 days" }),
    ).toHaveAttribute("aria-pressed", "false");
  });

  it("says the end date is not counted, where the dates are picked", async () => {
    render(
      <RangePicker offered={["P30D"]} selected="P30D" onSelect={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: /custom/i }));

    expect(screen.getByText(/end date is not counted/i)).toBeVisible();
  });
});

describe("<RangePicker> custom interval", () => {
  it("applies the span the calendar is showing", async () => {
    const onSelect = vi.fn();
    render(
      <RangePicker
        offered={["P30D"]}
        selected="2026-08-01/2026-09-01"
        onSelect={onSelect}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /2026-08-01/ }));

    await userEvent.click(screen.getByRole("button", { name: "Apply" }));

    expect(onSelect).toHaveBeenCalledWith("2026-08-01/2026-09-01");
  });

  it("has nothing to apply until a whole span is picked", async () => {
    render(
      <RangePicker offered={["P30D"]} selected="P30D" onSelect={vi.fn()} />,
    );

    await userEvent.click(screen.getByRole("button", { name: /custom/i }));

    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
  });

  it("leaves the window alone when the calendar is dismissed", async () => {
    const onSelect = vi.fn();
    render(
      <RangePicker offered={["P30D"]} selected="P30D" onSelect={onSelect} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /custom/i }));

    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onSelect).not.toHaveBeenCalled();
  });
});

describe("<RangePicker> reopened after the selection moved", () => {
  it("applies the interval it is showing now, not the one it opened with", async () => {
    const onSelect = vi.fn();
    const { rerender } = render(
      <RangePicker
        offered={["P30D"]}
        selected="2026-08-01/2026-09-01"
        onSelect={onSelect}
      />,
    );

    rerender(
      <RangePicker
        offered={["P30D"]}
        selected="2026-06-01/2026-07-01"
        onSelect={onSelect}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /2026-06-01/ }));
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));

    expect(onSelect).toHaveBeenCalledWith("2026-06-01/2026-07-01");
  });
});
