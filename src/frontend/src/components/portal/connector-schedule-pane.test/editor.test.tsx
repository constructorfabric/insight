import { mocks, read, pane, editCustom } from "./fixtures";

import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

import { AnalyticsApiError } from "@/api/analytics-client";

it.each(["inherited", "pending", "error"] as const)(
  "prefills applied crons and hides Reset after a %s clear",
  async (status) => {
    mocks.query.data!.schedule.status = status;
    mocks.query.data!.schedule.requested!.crons = [];
    mocks.query.data!.schedule.applied!.crons = ["0 3 * * *", "0 15 * * *"];
    mocks.query.data!.schedule.inherited = {
      crons: ["0 2 * * *"],
      origin: "annotation",
    };
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));

    expect(screen.getByRole("combobox")).toHaveValue("custom");
    expect(
      screen.getByRole("textbox", { name: "Cron expression 1" })
    ).toHaveValue("0 3 * * *");
    expect(
      screen.getByRole("textbox", { name: "Cron expression 2" })
    ).toHaveValue("0 15 * * *");
    expect(
      screen.queryByRole("button", { name: /Reset to/ })
    ).not.toBeInTheDocument();
    if (status === "error") {
      mocks.save.mockResolvedValue(read());
      await userEvent.click(screen.getByRole("button", { name: "Save" }));
      expect(mocks.save).toHaveBeenCalledWith({ etag: '"2"', crons: null });
    } else {
      expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    }
  }
);

it("prefills Custom from the compiled preset and disables Save until the ordered list changes", async () => {
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const save = screen.getByRole("button", { name: "Save" });
  expect(save).toBeDisabled();
  await userEvent.selectOptions(screen.getByRole("combobox"), "custom");
  const field = screen.getByRole("textbox", { name: "Cron expression 1" });
  expect(field).toHaveValue("0 6 * * *");
  expect(save).toBeDisabled();
  fireEvent.change(field, { target: { value: "0 7 * * *" } });
  expect(save).toBeEnabled();
  fireEvent.change(field, { target: { value: "0 6 * * *" } });
  expect(save).toBeDisabled();
  fireEvent.submit(field.closest("form")!);
  expect(mocks.save).not.toHaveBeenCalled();
  const help = screen.getByRole("link", { name: "Syntax help ↗" });
  expect(help).toHaveAttribute("href", "https://crontab.guru");
  expect(help).toHaveAttribute("target", "_blank");
  expect(help).toHaveAttribute("rel", "noopener noreferrer");
  expect(
    screen.getByText(/UTC · minute hour day month weekday/)
  ).toBeInTheDocument();
});

it("opens every stored expression as a row and warns before a preset replaces them", async () => {
  const crons = ["0 3 * * *", "0 12 * * 1-5", "30 18 * * 0"];
  mocks.query.data!.schedule.requested!.crons = crons;
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByRole("combobox")).toHaveValue("custom");
  expect(
    screen
      .getAllByRole("textbox")
      .map((input) => (input as HTMLInputElement).value)
  ).toEqual(crons);
  for (const reading of [
    "At 03:00 UTC",
    "At 12:00, Monday through Friday UTC",
    "At 18:30, only on Sunday UTC",
  ])
    expect(screen.getByText(reading), reading).toBeInTheDocument();
  expect(
    screen.getByText(
      "This schedule has 3 expressions. Choosing a preset replaces all 3."
    )
  ).toBeInTheDocument();
  expect(
    screen.getByText("Syncs on 3 expressions (all UTC).")
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await userEvent.selectOptions(screen.getByRole("combobox"), "daily");
  expect(
    screen.getByText(/Choosing a preset replaces all 3/)
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
});

it("adds and removes independent rows and stops at ten", async () => {
  render(pane());
  await editCustom("0 7 * * *");
  const add = screen.getByRole("button", { name: "+ Add expression" });
  for (let count = 1; count < 10; count++) await userEvent.click(add);
  expect(screen.getAllByRole("textbox")).toHaveLength(10);
  expect(add).toBeDisabled();
  expect(screen.getByText("Up to 10 expressions")).toBeInTheDocument();
  expect(
    screen.getAllByRole("button", { name: /Remove expression/ })
  ).toHaveLength(10);
  await userEvent.click(
    screen.getByRole("button", { name: "Remove expression 2" })
  );
  expect(screen.getAllByRole("textbox")).toHaveLength(9);
  expect(
    screen.getByRole("textbox", { name: "Cron expression 1" })
  ).toHaveValue("0 7 * * *");
  expect(add).toBeEnabled();
});

it.each([1, 2, 3])(
  "returns keyboard focus to an adjacent input after removing row %s",
  async (removed) => {
    mocks.query.data!.schedule.requested!.crons = [
      "0 3 * * *",
      "0 12 * * *",
      "0 18 * * *",
    ];
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    screen
      .getByRole("button", { name: `Remove expression ${removed}` })
      .focus();
    await userEvent.keyboard("{Enter}");
    expect(
      screen.getByRole("textbox", {
        name: `Cron expression ${Math.max(1, removed - 1)}`,
      })
    ).toHaveFocus();
  }
);

it.each([
  ["crons[1]", "hour: 25 is outside 0–23", true],
  [
    "crons[1]",
    'expressions 0 "0 7 * * *" and 1 "15 7 * * *" start 15 minutes apart',
    false,
  ],
  ["crons", "Too many starts in this schedule.", false],
] as const)(
  "places refusal %s %s at the proper scope",
  async (field, description, inline) => {
    mocks.query.data!.schedule.requested!.crons = ["0 7 * * *", "0 8 * * *"];
    mocks.save.mockRejectedValue(
      new AnalyticsApiError(400, {
        context: { field_violations: [{ field, description }] },
      })
    );
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const input = screen.getByRole("textbox", { name: "Cron expression 2" });
    fireEvent.change(input, { target: { value: "0 25 * * *" } });
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    const error = await screen.findByText(description);
    expect(input).toHaveValue("0 25 * * *");
    expect(input).toHaveAttribute("aria-invalid", String(inline));
    expect(
      screen.getByRole("textbox", { name: "Cron expression 1" })
    ).toHaveAttribute("aria-invalid", "false");
    if (inline) {
      expect(error.closest('[role="alert"]')).toBe(error);
      expect(input).toHaveAttribute("aria-describedby", error.id);
      expect(error).toHaveClass("text-destructive");
      expect(input).toHaveFocus();
      expect(error).toHaveAttribute("role", "alert");
    } else {
      expect(error.closest('[role="alert"]')).toBeInTheDocument();
      expect(error.closest('[role="alert"]')).toHaveFocus();
    }
  }
);

it("puts preset refusals above the footer without clearing the time", async () => {
  const description = "This schedule starts too often.";
  mocks.save.mockRejectedValue(
    new AnalyticsApiError(400, {
      context: { field_violations: [{ field: "crons[0]", description }] },
    })
  );
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByLabelText("Time"), {
    target: { value: "09:30" },
  });
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const error = await screen.findByText(description);
  expect(error.closest('[role="alert"]')).toBeInTheDocument();
  expect(error.closest('[role="alert"]')).toHaveFocus();
  expect(screen.getByLabelText("Time")).toHaveValue("09:30");
});

it("focuses a save failure alert once and lets the draft be edited again", async () => {
  mocks.save.mockRejectedValue(new Error("synthetic failure"));
  render(pane());
  const input = await editCustom("0 7 * * *");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveFocus();
  expect(alert).toHaveTextContent(
    "Could not save the schedule. Your input is kept; try again."
  );
  await userEvent.click(input);
  await userEvent.type(input, "1");
  expect(input).toHaveFocus();
});

it.each([
  ["inherited", "Inherited", false],
  ["applied", "Applied", true],
  ["pending", "● Pending", true],
  ["error", "Error", true],
] as const)(
  "keeps the header chip on one line for %s in both views",
  async (status, label, requested) => {
    mocks.query.data!.schedule.status = status;
    if (!requested) mocks.query.data!.schedule.requested = null;
    render(pane());
    for (let view = 0; view < 2; view++) {
      const text = screen.getByText(label);
      const chip = text.closest("[title]")!;
      const slot = chip.closest('[data-slot="schedule-header-status"]');
      expect.soft(slot).not.toHaveClass("h-12");
      expect.soft(slot!.parentElement).not.toHaveClass("h-14");
      expect.soft(chip).toHaveClass("whitespace-nowrap");
      expect.soft(text).toHaveClass("truncate");
      expect.soft(chip).toHaveAttribute("title", text.textContent);
      if (status === "error") expect(chip).toHaveClass("text-warning");
      if (view === 0)
        await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    }
  }
);

it("renders compact UTC read lines with attribution, and Edit at the bottom", () => {
  mocks.query.data!.schedule.status = "applied";
  render(pane());
  for (const label of ["Runs", "Set by"])
    expect(screen.getByText(label).tagName).toBe("DT");
  expect(screen.getByText(/Syncs every day at/)).toBeInTheDocument();
  expect(screen.getByText("Set by").nextElementSibling).toHaveTextContent(
    /you, Fri 16 Jan.*02:00/
  );
  expect(
    screen.getByRole("button", { name: "Edit" }).parentElement
  ).toHaveClass("justify-end");
  expect(screen.queryByText("Requested")).not.toBeInTheDocument();
});

it("groups the UTC summary and stored cron in one tinted preview", async () => {
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByLabelText("Time"), {
    target: { value: "09:00" },
  });
  const summary = screen.getByText("Syncs every day at 09:00 UTC.");
  const preview = summary.parentElement!;
  expect(preview).toHaveClass("bg-muted", "min-w-0");
  expect(within(preview).getByText(/Stored as/)).toHaveTextContent(
    "Stored as 0 9 * * * UTC."
  );
  await userEvent.selectOptions(screen.getByRole("combobox"), "custom");
  expect(screen.getAllByText("At 09:00 UTC")).toHaveLength(2);
  expect(screen.queryByText(/Stored as/)).not.toBeInTheDocument();
});

it("preserves custom row whitespace on open so Save stays disabled", async () => {
  mocks.query.data!.schedule.requested!.crons = [" 0 6 * * * "];
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByRole("combobox")).toHaveValue("custom");
  expect(screen.getByRole("textbox")).toHaveValue(" 0 6 * * * ");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

it.each([
  ["Asia/Kathmandu", -345],
  ["America/St_Johns", 210],
] as const)(
  "presets save UTC crons in viewer timezone %s",
  async (timeZone, offset) => {
    const options = new Intl.DateTimeFormat().resolvedOptions();
    const zone = vi
      .spyOn(Intl.DateTimeFormat.prototype, "resolvedOptions")
      .mockReturnValue({ ...options, timeZone });
    const timezoneOffset = vi
      .spyOn(Date.prototype, "getTimezoneOffset")
      .mockReturnValue(offset);
    try {
      for (const [preset, label, time, cron] of [
        ["hours-1", "Minute", "15", "15 * * * *"],
        [
          "hours-2",
          "Start time",
          "09:15",
          "15 1,3,5,7,9,11,13,15,17,19,21,23 * * *",
        ],
        ["hours-3", "Start time", "09:15", "15 0,3,6,9,12,15,18,21 * * *"],
        ["hours-4", "Start time", "09:15", "15 1,5,9,13,17,21 * * *"],
        ["hours-6", "Start time", "09:15", "15 3,9,15,21 * * *"],
        ["hours-8", "Start time", "09:15", "15 1,9,17 * * *"],
        ["hours-12", "Start time", "09:15", "15 9,21 * * *"],
        ["daily", "Time", "09:15", "15 9 * * *"],
        ["weekdays", "Time", "00:15", "15 0 * * 1-5"],
      ]) {
        mocks.save.mockResolvedValue(read());
        render(pane());
        await userEvent.click(screen.getByRole("button", { name: "Edit" }));
        await userEvent.selectOptions(screen.getByRole("combobox"), preset);
        fireEvent.change(screen.getByLabelText(label), {
          target: { value: time },
        });
        expect(screen.getByText("UTC"), preset).toBeInTheDocument();
        expect(screen.getByText(/Stored as/), preset).toHaveTextContent(
          `Stored as ${cron} UTC.`
        );
        await userEvent.click(screen.getByRole("button", { name: "Save" }));
        expect(mocks.save, preset).toHaveBeenLastCalledWith({
          etag: '"2"',
          crons: [cron],
        });
        cleanup();
      }
    } finally {
      zone.mockRestore();
      timezoneOffset.mockRestore();
    }
  }
);
