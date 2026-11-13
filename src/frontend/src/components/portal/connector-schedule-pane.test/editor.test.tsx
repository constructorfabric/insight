import { pickPreset, presetLabel } from "./preset";
import { mocks, read, pane, editCustom, pickTime } from "./fixtures";

import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

import { ConnectorsApiError } from "@/api/connectors-client";

it.each(["inherited", "pending"] as const)(
  "prefills applied crons and hides Reset after a %s clear",
  async (status) => {
    mocks.query.data!.schedule.status = status;
    mocks.query.data!.schedule.requested!.crons = [];
    mocks.query.data!.schedule.live!.crons = ["0 3 * * *", "0 15 * * *"];
    mocks.query.data!.schedule.inherited = {
      crons: ["0 2 * * *"],
      origin: "annotation",
    };
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));

    expect(
      within(screen.getByRole("combobox", { name: "Runs" })).getByText(
        presetLabel("custom"),
        { exact: true }
      )
    ).toBeInTheDocument();
    expect(
      screen.getByRole("textbox", { name: "Cron expression 1" })
    ).toHaveValue("0 3 * * *");
    expect(
      screen.getByRole("textbox", { name: "Cron expression 2" })
    ).toHaveValue("0 15 * * *");
    expect(
      screen.queryByRole("button", { name: /Reset to/ })
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(mocks.save).not.toHaveBeenCalled();
  }
);

it("prefills Custom from the compiled preset and disables Save until the ordered list changes", async () => {
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const save = screen.getByRole("button", { name: "Save" });
  expect(save).toBeDisabled();
  await pickPreset("custom");
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
  expect(
    within(screen.getByRole("combobox", { name: "Runs" })).getByText(
      presetLabel("custom"),
      { exact: true }
    )
  ).toBeInTheDocument();
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
    expect(screen.queryByText(reading), reading).not.toBeInTheDocument();
  expect(screen.getByText("Replaces all 3 expressions")).toBeInTheDocument();
  expect(
    screen.queryByText("Syncs on 3 expressions (all UTC).")
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  await pickPreset("daily");
  expect(screen.getByText("Replaces all 3 expressions")).toBeInTheDocument();
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
      new ConnectorsApiError(400, {
        context: { field_violations: [{ field, description }] },
      })
    );
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    const input = screen.getByRole("textbox", { name: "Cron expression 2" });
    fireEvent.change(input, { target: { value: "0 9 * * *" } });
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    const error = await screen.findByText(description);
    expect(input).toHaveValue("0 9 * * *");
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
    new ConnectorsApiError(400, {
      context: { field_violations: [{ field: "crons[0]", description }] },
    })
  );
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  await pickTime("09:30");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const error = await screen.findByText(description);
  expect(error.closest('[role="alert"]')).toBeInTheDocument();
  expect(error.closest('[role="alert"]')).toHaveFocus();
  expect(screen.getByRole("combobox", { name: "Hour" })).toHaveTextContent(
    "09"
  );
  expect(screen.getByRole("combobox", { name: "Minute" })).toHaveTextContent(
    "30"
  );
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
  ["inherited", "Default", false],
  ["applied", "Applied", true],
  ["pending", "Waiting for reconcile", true],
  ["pending", "Waiting for reconcile", true],
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
      if (status === "pending") expect(chip).toHaveClass("text-warning");
      if (view === 0)
        await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    }
  }
);

it("renders compact UTC values with relative attribution and Edit in the header", () => {
  mocks.query.data!.schedule.status = "applied";
  render(pane());
  expect(screen.getByText("Every day at 03:00")).toBeInTheDocument();
  expect(screen.getByText("0 3 * * *").parentElement).toHaveTextContent("UTC");
  expect(screen.getByText("set by you · just now")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Edit" }).closest("header")
  ).toContainElement(screen.getByRole("heading", { name: "Schedule" }));
  expect(screen.queryByText("Requested")).not.toBeInTheDocument();
});

it("groups the UTC cron and short run chips in one tinted preview", async () => {
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  await pickTime("09:00");
  const preview = screen.getByText("0 9 * * *").parentElement!.parentElement!;
  expect(preview).toHaveClass("bg-muted", "min-w-0");
  expect(
    within(preview).getByText("0 9 * * *").parentElement
  ).toHaveTextContent("UTC");
  expect(within(preview).getAllByRole("listitem")).toHaveLength(3);
  await pickPreset("custom");
  expect(screen.queryByText("At 09:00 UTC")).not.toBeInTheDocument();
  expect(screen.queryByText(/Stored as/)).not.toBeInTheDocument();
});

it("preserves custom row whitespace on open so Save stays disabled", async () => {
  mocks.query.data!.schedule.requested!.crons = [" 0 6 * * * "];
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(
    within(screen.getByRole("combobox", { name: "Runs" })).getByText(
      presetLabel("custom"),
      { exact: true }
    )
  ).toBeInTheDocument();
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
        await pickPreset(preset);
        if (label === "Minute") {
          fireEvent.change(screen.getByLabelText(label), {
            target: { value: time },
          });
        } else {
          await pickTime(time);
        }
        expect(screen.getAllByText("UTC").length, preset).toBeGreaterThan(0);
        expect(screen.getByText(cron), preset).toHaveTextContent(cron);
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

it.each(["save", "reset"])(
  "shows the server's not-changed reason and reloads what runs after a refused %s",
  async (operation) => {
    const reason =
      operation === "save"
        ? "The schedule was not changed: synthetic cluster refusal"
        : "The schedule was not changed: Reset requires the deployment schedule annotation";
    const running = read();
    running.schedule.status = "applied";
    running.schedule.live!.crons = ["0 6 * * *"];
    mocks.query.refetch.mockResolvedValue({ data: running });
    mocks.save.mockRejectedValue(
      new ConnectorsApiError(503, { detail: reason })
    );
    render(pane());
    await editCustom("0 7 * * *");
    await userEvent.click(
      screen.getByRole("button", {
        name: operation === "save" ? "Save" : /Reset to/,
      })
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(reason);
    expect(mocks.query.refetch).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("combobox", { name: "Hour" })).toHaveTextContent(
      "06"
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(mocks.toast).not.toHaveBeenCalled();
  }
);
