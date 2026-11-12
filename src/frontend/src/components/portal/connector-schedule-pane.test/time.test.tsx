import { presetLabel } from "./preset";
import { mocks, pane, read } from "./fixtures";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

it.each([
  [
    "hours-2",
    "Start time",
    "19 1,3,5,7,9,11,13,15,17,19,21,23 * * *",
    "01",
    "19 0,2,4,6,8,10,12,14,16,18,20,22 * * *",
    "30 0,2,4,6,8,10,12,14,16,18,20,22 * * *",
    "2026-01-15T12:19:00.000Z",
    [
      "2026-01-15T12:30:00.000Z",
      "2026-01-15T14:30:00.000Z",
      "2026-01-15T16:30:00.000Z",
    ],
  ],
  [
    "daily",
    "Time",
    "19 12 * * *",
    "12",
    "19 14 * * *",
    "30 14 * * *",
    "2026-01-15T14:19:00.000Z",
    [
      "2026-01-15T14:30:00.000Z",
      "2026-01-16T14:30:00.000Z",
      "2026-01-17T14:30:00.000Z",
    ],
  ],
  [
    "weekdays",
    "Time",
    "19 12 * * 1-5",
    "12",
    "19 14 * * 1-5",
    "30 14 * * 1-5",
    "2026-01-15T14:19:00.000Z",
    [
      "2026-01-15T14:30:00.000Z",
      "2026-01-16T14:30:00.000Z",
      "2026-01-19T14:30:00.000Z",
    ],
  ],
] as const)(
  "renders stored %s time in kit selects and updates UTC cron, runs and Save",
  async (
    preset,
    context,
    stored,
    initialHour,
    afterHour,
    finalCron,
    nextAfterHour,
    nextRuns
  ) => {
    mocks.query.data!.schedule.requested!.crons = [stored];
    mocks.save.mockResolvedValue(read());
    const { container } = render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(
      within(screen.getByRole("combobox", { name: "Runs" })).getByText(
        presetLabel(preset),
        { exact: true }
      )
    ).toBeInTheDocument();
    const group = screen.getByRole("group", { name: context });
    const hour = within(group).getByRole("combobox", { name: "Hour" });
    const minute = within(group).getByRole("combobox", { name: "Minute" });
    expect(hour).toHaveTextContent(initialHour);
    expect(minute).toHaveTextContent("19");
    expect(hour).toHaveClass("tabular-nums");
    expect(minute).toHaveClass("tabular-nums");
    expect(container.querySelector('input[type="time"]')).toBeNull();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await userEvent.click(hour);
    const hours = within(await screen.findByRole("listbox")).getAllByRole(
      "option"
    );
    expect(hours.map((option) => option.textContent)).toEqual(
      Array.from({ length: 24 }, (_, index) => String(index).padStart(2, "0"))
    );
    await userEvent.click(screen.getByRole("option", { name: "14" }));
    expect(hour).toHaveTextContent("14");
    expect(minute).toHaveTextContent("19");
    expect(screen.getByText(afterHour)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    expect(
      screen.getByText("Next (UTC)").parentElement!.querySelector("time")
    ).toHaveAttribute("datetime", nextAfterHour);

    await userEvent.click(minute);
    const minutes = within(await screen.findByRole("listbox")).getAllByRole(
      "option"
    );
    expect(minutes.map((option) => option.textContent)).toEqual(
      Array.from({ length: 60 }, (_, index) => String(index).padStart(2, "0"))
    );
    await userEvent.click(screen.getByRole("option", { name: "30" }));
    expect(hour).toHaveTextContent("14");
    expect(minute).toHaveTextContent("30");
    expect(screen.getByText(finalCron)).toBeInTheDocument();
    const times = screen
      .getByText("Next (UTC)")
      .parentElement!.querySelectorAll("time");
    expect(Array.from(times, (time) => time.dateTime)).toEqual(nextRuns);
    expect(times[0]).toHaveTextContent(nextRuns[0].slice(11, 16));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(mocks.save).toHaveBeenCalledWith({
      etag: '"2"',
      crons: [finalCron],
    });
  }
);

it.each(["{Enter}", " ", "{ArrowDown}"])(
  "opens time choices with %s and Escape closes only the popup",
  async (key) => {
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const hour = screen.getByRole("combobox", { name: "Hour" });
    hour.focus();
    await userEvent.keyboard(key);
    expect(screen.getByRole("listbox")).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeInTheDocument();
    expect(hour).toHaveFocus();
  }
);

it("uses the kit type-ahead to choose an hour", async () => {
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const hour = screen.getByRole("combobox", { name: "Hour" });
  hour.focus();
  await userEvent.keyboard("{Enter}14{Enter}");
  expect(hour).toHaveTextContent("14");
  expect(screen.getByText("0 14 * * *")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
});
