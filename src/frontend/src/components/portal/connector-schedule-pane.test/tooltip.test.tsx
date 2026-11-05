import { mocks, pane } from "./fixtures";

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

it("omits saved-by attribution from a pending clear tooltip", async () => {
  mocks.query.data!.schedule.requested!.crons = [];
  render(pane());
  await userEvent.hover(screen.getByText("● Pending").closest("[tabindex]")!);

  const tooltip = await screen.findByRole("tooltip");
  expect(tooltip).toHaveTextContent("Request revision 2");
  expect(tooltip).not.toHaveTextContent("saved by");
});

it("explains the header chip on hover and keyboard focus with separate stacked lines", async () => {
  mocks.query.data!.schedule.status = "inherited";
  mocks.query.data!.schedule.requested = null;
  mocks.query.data!.schedule.inherited = {
    crons: ["0 2 * * *", "0 14 * * *"],
    origin: "annotation",
  };
  render(pane());
  const trigger = screen.getByText("Inherited").closest("[tabindex]")!;
  const user = userEvent.setup();
  for (const gesture of ["hover", "focus"]) {
    if (gesture === "hover") await user.hover(trigger);
    else {
      await user.tab();
      expect(trigger).toHaveFocus();
    }
    const tooltip = await screen.findByRole("tooltip");
    expect(trigger).toHaveAttribute("aria-describedby", tooltip.id);
    const lines = Array.from(tooltip.querySelectorAll("span"));
    expect(
      lines.map((line) => line.textContent),
      gesture
    ).toEqual([
      "No schedule request. Runs the deployment schedule from the Secret annotation insight.cyberfabric.com/schedule.",
      "0 2 * * * UTC",
      "0 14 * * * UTC",
    ]);
    expect(lines[0].parentElement).toHaveClass("flex", "flex-col");
    for (const line of lines) expect(line).toHaveClass("block");
    await user.unhover(trigger);
    if (gesture === "focus") await user.tab();
    await waitFor(() =>
      expect(screen.queryByRole("tooltip")).not.toBeInTheDocument()
    );
  }
});
