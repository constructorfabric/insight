/**
 * Component coverage for `<ComingSoon>`: all three variants (card / chip /
 * row) x states (empty / error / loading), custom labels, and the retry
 * button that only appears for `state="error"` with an `onRetry` handler.
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { ComingSoon } from "./coming-soon";

describe("<ComingSoon>", () => {
  it("renders the card variant with the default empty label", () => {
    render(<ComingSoon />);
    expect(
      screen.getByRole("status", { name: "No data for this period" }),
    ).toBeInTheDocument();
    expect(screen.getByText("No data for this period")).toBeInTheDocument();
  });

  it("renders the loading state label", () => {
    render(<ComingSoon state="loading" />);
    expect(
      screen.getByRole("status", { name: "Loading…" }),
    ).toBeInTheDocument();
  });

  it("renders a custom label over the state default", () => {
    render(<ComingSoon label="Nothing here yet" />);
    expect(
      screen.getByRole("status", { name: "Nothing here yet" }),
    ).toBeInTheDocument();
  });

  it("card error state shows a Retry button wired to onRetry", async () => {
    const onRetry = vi.fn();
    render(<ComingSoon state="error" onRetry={onRetry} />);
    expect(screen.getByText("Unable to load")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("hides Retry when errored without an onRetry handler", () => {
    render(<ComingSoon state="error" />);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
  });

  it("hides Retry for non-error states even with onRetry", () => {
    render(<ComingSoon state="empty" onRetry={vi.fn()} />);
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
  });

  it("renders the chip variant with retry in the error state", async () => {
    const onRetry = vi.fn();
    render(<ComingSoon variant="chip" state="error" onRetry={onRetry} />);
    const chip = screen.getByRole("status", { name: "Unable to load" });
    expect(chip.tagName).toBe("SPAN");
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("renders the chip variant without retry when empty", () => {
    render(<ComingSoon variant="chip" />);
    expect(
      screen.getByRole("status", { name: "No data for this period" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("renders the row variant with retry in the error state", async () => {
    const onRetry = vi.fn();
    render(<ComingSoon variant="row" state="error" onRetry={onRetry} />);
    expect(
      screen.getByRole("status", { name: "Unable to load" }),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("renders the row variant with the default empty label", () => {
    render(<ComingSoon variant="row" />);
    expect(
      screen.getByRole("status", { name: "No data for this period" }),
    ).toBeInTheDocument();
  });
});
