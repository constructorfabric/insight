/**
 * Component coverage for `<PeriodSelectorBar>`: period tab switching, the
 * custom-range trigger label, and the calendar popover's Apply / Cancel /
 * Clear actions. A fixed custom range keeps date labels deterministic.
 */

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CustomRange } from "@/types/insight";

import { PeriodSelectorBar } from "./period-selector-bar";

const CUSTOM: CustomRange = { from: "2026-07-01", to: "2026-07-10" };

function renderBar(customRange: CustomRange | null = null) {
  const onPeriodChange = vi.fn();
  const onRangeChange = vi.fn();
  render(
    <PeriodSelectorBar
      period="month"
      customRange={customRange}
      onPeriodChange={onPeriodChange}
      onRangeChange={onRangeChange}
    />,
  );
  return { onPeriodChange, onRangeChange };
}

describe("<PeriodSelectorBar>", () => {
  it("renders the four period tabs", () => {
    renderBar();
    for (const label of ["Week", "Month", "Quarter", "Year"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("fires onPeriodChange when a period tab is clicked", async () => {
    const { onPeriodChange } = renderBar();
    await userEvent.click(screen.getByText("Week"));
    expect(onPeriodChange).toHaveBeenCalledWith("week");
  });

  it("shows the active custom range on the calendar trigger", () => {
    renderBar(CUSTOM);
    expect(screen.getByText("1 Jul – 10 Jul")).toBeInTheDocument();
    expect(screen.getByText("UTC")).toBeInTheDocument();
  });

  it("opens the calendar popover and cancels without applying", { timeout: 30_000 }, async () => {
    const { onRangeChange } = renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    // Header echoes the pre-seeded range in long form.
    expect(
      await screen.findByText(/1 Jul 2026\s*–\s*10 Jul 2026/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "Apply" }),
      ).not.toBeInTheDocument();
    });
    expect(onRangeChange).not.toHaveBeenCalled();
  });

  it("applies the pre-seeded range through the Apply button", { timeout: 30_000 }, async () => {
    const { onRangeChange } = renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    const apply = await screen.findByRole("button", { name: "Apply" });
    expect(apply).toBeEnabled();
    await userEvent.click(apply);
    expect(onRangeChange).toHaveBeenCalledWith(CUSTOM);
  });

  it("clears an active custom range", { timeout: 30_000 }, async () => {
    const { onRangeChange } = renderBar(CUSTOM);
    await userEvent.click(screen.getByText("1 Jul – 10 Jul"));
    await userEvent.click(
      await screen.findByRole("button", { name: "Clear" }),
    );
    expect(onRangeChange).toHaveBeenCalledWith(null);
  });

  it("hides Clear when no custom range is active", { timeout: 30_000 }, async () => {
    renderBar(null);
    // Without a custom range the trigger carries the resolved period range.
    await userEvent.click(screen.getByText("UTC").closest("button")!);
    await screen.findByRole("button", { name: "Apply" });
    expect(
      screen.queryByRole("button", { name: "Clear" }),
    ).not.toBeInTheDocument();
  });
});
