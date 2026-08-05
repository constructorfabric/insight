import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { DashboardEmptyState } from "@/components/widgets/dashboard/dashboard-empty-state";

describe("DashboardEmptyState", () => {
  it("offers every wider period and reports the current one", () => {
    render(<DashboardEmptyState period="week" onSetPeriod={() => {}} />);

    expect(screen.getByText("No activity in past week")).toBeInTheDocument();
    expect(
      screen.getByText(/Nothing has been recorded in this period\. Try a wider one\./),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try past month" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try past quarter" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try past year" })).toBeInTheDocument();
  });

  it("invokes onSetPeriod with the picked period", async () => {
    const user = userEvent.setup();
    const onSetPeriod = vi.fn();
    render(<DashboardEmptyState period="month" onSetPeriod={onSetPeriod} />);

    await user.click(screen.getByRole("button", { name: "Try past quarter" }));
    expect(onSetPeriod).toHaveBeenCalledWith("quarter");
  });

  it("renders no suggestions for the widest period", () => {
    render(<DashboardEmptyState period="year" onSetPeriod={() => {}} />);

    expect(screen.getByText("No activity in past year")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByText(/Try a wider one/)).not.toBeInTheDocument();
  });
});
