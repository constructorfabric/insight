import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

vi.mock("@/queries/connector-health", () => ({
  useConnectorHealth: () => ({
    data: {
      as_of: "2026-01-15T12:00:00Z",
      history_available: true,
      connectors: [
        {
          connector: "example-tracker",
          source_id: "main",
          configured: true,
          schedule_request: {
            status: "pending",
            revision: 3,
            crons: ["0 6 * * *"],
            updated_by: "user@example.com",
            updated_at: "2026-01-15T12:00:00Z",
            applied: { revision: 2, crons: ["0 3 * * *"] },
          },
        },
      ],
    },
  }),
}));

import { ConnectorHealthPane } from "./connector-health";
import { AppTooltips } from "@/test/tooltip";

afterEach(() => vi.useRealTimers());

it("explains the collapsed chip on hover and keyboard focus with separate stacked lines", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime("2026-01-15T12:00:00Z");
  render(<ConnectorHealthPane />, { wrapper: AppTooltips });
  const row = screen.getByRole("row", { expanded: false });
  const slot = row.querySelector('[data-slot="schedule-request-status"]')!;
  const trigger = within(slot as HTMLElement)
    .getByText("change pending")
    .closest<HTMLElement>("[tabindex]")!;
  const user = userEvent.setup();
  for (const gesture of ["hover", "focus"]) {
    if (gesture === "hover") await user.hover(trigger);
    else {
      await user.tab();
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
      "Request revision 3, saved by user@example.com at 2026-01-15 12:00:00Z.",
      "Revision 2 is running now.",
      "0 3 * * * UTC",
    ]);
    expect(lines[0].parentElement).toHaveClass("flex", "flex-col");
    for (const line of lines) expect(line).toHaveClass("block");
    expect(row).toHaveAttribute("aria-expanded", "false");
    await user.click(trigger);
    expect(row).toHaveAttribute("aria-expanded", "false");
    await user.unhover(trigger);
    await user.tab();
    await waitFor(() =>
      expect(screen.queryByRole("tooltip")).not.toBeInTheDocument()
    );
  }
});
