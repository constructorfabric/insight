import { mocks, row, NOW, read, pane, editCustom } from "./fixtures";

import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { AnalyticsApiError } from "@/api/analytics-client";
import { localTime } from "@/lib/portal/connector-schedule-editor";
import { ConnectorSchedulePane } from "../connector-schedule-pane";

it("shows Edit when the loaded schedule response has no editable field", async () => {
  expect(mocks.query.data!.schedule).not.toHaveProperty("editable");
  render(pane());

  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByRole("combobox", { name: "Runs" })).toHaveFocus();
});

it("wraps schedule values, attribution and status inside a nowrap table cell", () => {
  render(<div className="whitespace-nowrap">{pane()}</div>);

  const content = screen.getByRole("heading", { name: "Schedule" })
    .parentElement!.parentElement!;
  expect(content).toHaveClass(
    "min-w-0",
    "whitespace-normal",
    "[overflow-wrap:anywhere]"
  );
  expect(content).not.toHaveClass("whitespace-nowrap");
  for (const text of [/0 3 \* \* \*/, /· you,/]) {
    expect(screen.getByText(text).closest("dd"), String(text)).toHaveClass(
      "min-w-0"
    );
  }
  expect(screen.getByText("Runs now").parentElement).toHaveClass(
    "grid-cols-[auto_minmax(0,1fr)]"
  );
  expect(
    screen
      .getByText(/Saved. Until the request is applied/)
      .closest("[aria-live]")
  ).not.toHaveClass("whitespace-nowrap");
});

it("contains the custom field and wraps editor errors and help inside a nowrap table cell", async () => {
  const description =
    "The expression is invalid. Choose starts at least 120 minutes apart and save the schedule again.";
  mocks.save.mockRejectedValue(
    new AnalyticsApiError(400, {
      context: { field_violations: [{ description }] },
    })
  );
  render(<div className="whitespace-nowrap">{pane()}</div>);
  const field = await editCustom();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText(description);

  const form = field.closest("form");
  expect(form).toHaveClass(
    "min-w-0",
    "whitespace-normal",
    "[overflow-wrap:anywhere]"
  );
  expect(field.closest("fieldset")).toHaveClass("min-w-0");
  expect(field).toHaveClass("w-[24ch]", "min-w-0", "max-w-full");
  for (const text of [description, /UTC · minute hour day month weekday/]) {
    expect(screen.getByText(text).closest("form"), String(text)).toBe(form);
    expect(screen.getByText(text), String(text)).not.toHaveClass(
      "whitespace-nowrap"
    );
  }
});

it.each([
  ["hours-1", "at minute", "Minute", "number"],
  ["hours-6", "starting", "Start time", "time"],
  ["hours-2", "starting", "Start time", "time"],
  ["hours-3", "starting", "Start time", "time"],
  ["hours-4", "starting", "Start time", "time"],
  ["hours-8", "starting", "Start time", "time"],
  ["hours-12", "starting", "Start time", "time"],
  ["daily", "at", "Time", "time"],
  ["weekdays", "at", "Time", "time"],
] as const)(
  "shows a compact inline field for %s",
  async (value, connector, label, type) => {
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("combobox", { name: "Runs" })).toHaveFocus();
    await userEvent.selectOptions(screen.getByRole("combobox"), value);
    const input = screen.getByLabelText(label);
    expect(input).toHaveAttribute("type", type);
    expect(input).toHaveClass(
      type === "time" ? "w-[88px]" : "w-[64px]",
      "h-8",
      "max-w-full",
      "min-w-0"
    );
    expect(
      within(input.parentElement!).getByText(connector)
    ).toBeInTheDocument();
    expect(
      within(input.parentElement!.parentElement!).getByText("UTC")
    ).toBeInTheDocument();
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
  }
);

it("keeps recorded and requested schedules separate and announces pending with attribution", () => {
  render(pane());
  expect(screen.getByText("Runs now")).toBeInTheDocument();
  expect(screen.getByText("Requested")).toBeInTheDocument();
  expect(screen.getByText(/0 3 \* \* \*/)).toBeInTheDocument();
  expect(screen.getByText(/0 6 \* \* \*/)).toBeInTheDocument();
  expect(screen.getByText(/· you,/)).toBeInTheDocument();
  expect(
    screen
      .getByText(/Saved. Until the request is applied/)
      .closest("[aria-live]")
  ).toHaveAttribute("aria-live", "polite");
});

it.each([false, true])(
  "shows the acknowledgement Error message and attempt time and retries the same request, cleared: %s",
  async (cleared) => {
    mocks.query.data!.schedule.status = "error";
    if (cleared) mocks.query.data!.schedule.requested!.crons = [];
    mocks.query.data!.schedule.error = {
      attempted_revision: 2,
      attempted_at: NOW,
      message: "Synthetic workflow update failed.",
    };
    mocks.query.data!.schedule.requested!.updated_by = "another-example-admin";
    mocks.save.mockResolvedValue(read());
    render(pane());
    expect(screen.getByText(/· another admin,/)).toBeInTheDocument();
    expect(screen.getByText("Error")).toBeInTheDocument();
    expect(
      screen
        .getByText("Synthetic workflow update failed.")
        .closest("[aria-live]")
    ).toHaveClass("text-warning");
    expect(screen.getByText(/Attempted at/)).toHaveTextContent(
      `Attempted at ${localTime(NOW)}. Edit and save again to retry.`
    );
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(mocks.save).toHaveBeenCalledWith({
      etag: '"2"',
      crons: cleared ? null : ["0 6 * * *"],
    });
  }
);

it.each(["pending", "error"] as const)(
  "dates an Applied transition from %s with the acknowledgement observed_at",
  (status) => {
    mocks.query.data!.schedule.status = status;
    const view = render(pane());
    const observedAt = "2026-01-15T12:17:00Z";
    mocks.query.data!.schedule.status = "applied";
    mocks.query.data!.schedule.applied!.observed_at = observedAt;
    view.rerender(pane());
    expect(screen.getByText("Applied")).toBeInTheDocument();
    expect(screen.getByText(/Applied at/)).toHaveTextContent(
      `Applied at ${localTime(observedAt)}.`
    );
  }
);

it.each(["pending", "error"] as const)(
  "omits Set by while a %s request differs",
  (status) => {
    mocks.query.data!.schedule.status = status;
    render(pane());
    expect(screen.queryByText("Set by")).not.toBeInTheDocument();
  }
);

it("shows applied only after a transition and keeps it until the row unmounts", () => {
  const view = render(pane());
  mocks.query.data!.schedule.status = "applied";
  view.rerender(pane());
  expect(screen.getByText(/Applied at/)).toBeInTheDocument();
  expect(screen.queryByText("Requested")).not.toBeInTheDocument();
  view.rerender(pane());
  expect(screen.getByText(/Applied at/)).toBeInTheDocument();
  view.unmount();
  render(pane());
  expect(screen.queryByText(/Applied at/)).not.toBeInTheDocument();
});

it("renders no recorded schedule as unknown and hides requested once inherited", () => {
  mocks.query.data!.schedule.status = "inherited";
  mocks.query.data!.schedule.requested = null;
  render(
    <ConnectorSchedulePane row={{ ...row, schedule: null }} polling={false} />
  );
  expect(screen.getAllByText("—")).toHaveLength(1);
  expect(screen.queryByText("Requested")).not.toBeInTheDocument();
});

it("shows the running schedule without override attribution after an inherited clear", () => {
  mocks.query.data!.schedule.status = "inherited";
  mocks.query.data!.schedule.requested!.crons = [];
  mocks.query.data!.schedule.inherited = {
    crons: ["0 3 * * *"],
    origin: "annotation",
  };
  render(pane());

  expect(screen.getByText("Runs").nextElementSibling).toHaveTextContent(
    "0 3 * * * (UTC)"
  );
  expect(screen.queryByText("Set by")).not.toBeInTheDocument();
  expect(screen.queryByText("Requested")).not.toBeInTheDocument();
  expect(
    screen.queryByText(/overridden by this request/)
  ).not.toBeInTheDocument();
});

it.each([
  ["pending", "annotation", "the Secret annotation"],
  ["pending", "descriptor", "the connector descriptor"],
  ["pending", "default", "the default"],
  ["error", "annotation", "the Secret annotation"],
  ["error", "descriptor", "the connector descriptor"],
  ["error", "default", "the default"],
] as const)(
  "names the deployment schedule and origin for a %s clear from %s",
  (status, origin, source) => {
    mocks.query.data!.schedule.status = status;
    mocks.query.data!.schedule.requested!.crons = [];
    mocks.query.data!.schedule.inherited = {
      crons: ["0 2 * * *", "0 14 * * *"],
      origin,
    };
    render(pane());

    const requested = screen.getByText("Requested").nextElementSibling!;
    expect(requested).toHaveTextContent(
      `Returns to the deployment schedule: Syncs on 2 expressions (all UTC). (0 2 * * *; 0 14 * * * UTC, from ${source})`
    );
    expect(requested).toHaveTextContent(`· you, ${localTime(NOW)}`);
    expect(screen.queryByText("The default schedule")).not.toBeInTheDocument();
    expect(screen.queryByText("Set by")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/overridden by this request/)
    ).not.toBeInTheDocument();
  }
);

it.each(["pending", "error"] as const)(
  "does not invent a deployment origin for a %s clear without inherited facts",
  (status) => {
    mocks.query.data!.schedule.status = status;
    mocks.query.data!.schedule.requested!.crons = [];
    render(pane());

    expect(screen.getByText("Requested").nextElementSibling).toHaveTextContent(
      `Returns to the deployment schedule. · you, ${localTime(NOW)}`
    );
    expect(screen.queryByText("The default schedule")).not.toBeInTheDocument();
  }
);

it("submits the custom list with the last ETag, then closes and restores focus", async () => {
  mocks.save.mockResolvedValue(read());
  render(pane());
  await editCustom("0 9 * * *");
  await userEvent.click(
    screen.getByRole("button", { name: "+ Add expression" })
  );
  await userEvent.type(
    screen.getByRole("textbox", { name: "Cron expression 2" }),
    "0 18 * * *"
  );
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mocks.save).toHaveBeenCalledWith({
      etag: '"2"',
      crons: ["0 9 * * *", "0 18 * * *"],
    })
  );
  expect(screen.getByRole("button", { name: "Edit" })).toHaveFocus();
  expect(
    screen.getByText(/Saved. Until the request is applied/)
  ).toBeInTheDocument();
});

it("shows every 400 description under Custom and keeps the input intact", async () => {
  mocks.save.mockRejectedValue(
    new AnalyticsApiError(400, {
      context: {
        field_violations: [
          { field: "crons[0]", description: "Minute field is invalid." },
          { field: "crons[0]", description: "Hour field is invalid." },
        ],
      },
    })
  );
  render(pane());
  await editCustom();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(
    await screen.findByText(/Minute field is invalid/)
  ).toBeInTheDocument();
  expect(screen.getByText(/Hour field is invalid/)).toBeInTheDocument();
  expect(screen.getByRole("textbox")).toHaveValue("not a cron");
  expect(screen.getByRole("textbox")).toHaveAttribute("aria-invalid", "true");
});

it("reloads a 412 into the open editor and uses the reloaded ETag on the next save", async () => {
  const next = read();
  next.etag = '"3"';
  next.schedule.requested = {
    crons: ["0 9 * * 0"],
    updated_at: "2026-01-15T12:29:00Z",
    updated_by: "another-example-admin",
  };
  mocks.query.refetch.mockResolvedValue({ data: next });
  mocks.save
    .mockRejectedValueOnce(new AnalyticsApiError(412, null))
    .mockResolvedValueOnce(next);
  render(pane());
  await editCustom("0 8 * * *");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(
    await screen.findByText(/Another admin changed this schedule at/)
  ).toHaveTextContent("It is reloaded below; check it and save again.");
  expect(screen.getByRole("textbox")).toHaveValue("0 9 * * 0");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "0 10 * * 0" },
  });
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await waitFor(() =>
    expect(mocks.save).toHaveBeenLastCalledWith({
      etag: '"3"',
      crons: ["0 10 * * 0"],
    })
  );
});

it("keeps input when a conflict reload fails", async () => {
  mocks.save.mockRejectedValue(new AnalyticsApiError(412, null));
  mocks.query.refetch.mockResolvedValue({
    error: new Error("synthetic failure"),
  });
  render(pane());
  await editCustom("0 8 * * *");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(
    await screen.findByText(/Could not reload the changed schedule/)
  ).toBeInTheDocument();
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * *");
});

it("keeps the input on other save failures", async () => {
  mocks.save.mockRejectedValue(new AnalyticsApiError(500, null));
  render(pane());
  await editCustom();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(
    await screen.findByText(/Could not save the schedule/)
  ).toBeInTheDocument();
  expect(screen.getByRole("textbox")).toHaveValue("not a cron");
});

it("resets with If-Match and closes after success", async () => {
  mocks.save.mockResolvedValue(read());
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  await userEvent.click(
    screen.getByRole("button", { name: "Reset to the default schedule" })
  );
  expect(mocks.save).toHaveBeenCalledWith({ etag: '"2"', crons: null });
  expect(await screen.findByRole("button", { name: "Edit" })).toHaveFocus();
});

it.each(["Cancel", "Escape"])(
  "%s cancels without writing and returns focus to Edit",
  async (method) => {
    render(pane());
    const field = await editCustom();
    if (method === "Cancel")
      await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    else fireEvent.keyDown(field, { key: "Escape" });
    expect(mocks.save).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Edit" })).toHaveFocus();
  }
);

it("prefills the compiled request", async () => {
  mocks.query.data!.schedule.requested!.crons = ["0 0,6,12,18 * * *"];
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByRole("combobox")).toHaveValue("hours-6");
});

it("prefills the recorded schedule without a request and hides Reset", async () => {
  mocks.query.data!.schedule.requested = null;
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  expect(screen.getByRole("combobox")).toHaveValue("daily");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  expect(
    screen.queryByRole("button", { name: "Reset to the default schedule" })
  ).not.toBeInTheDocument();
});

it("offers retry after a schedule read error", async () => {
  mocks.query.isError = true;
  mocks.query.data = undefined;
  render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(mocks.query.refetch).toHaveBeenCalled();
});

it.each([
  ["annotation", "the Secret annotation"],
  ["descriptor", "the connector descriptor"],
  ["default", "the default"],
] as const)(
  "shows the overridden deployment schedule from %s",
  (origin, source) => {
    mocks.query.data!.schedule.inherited = {
      crons: ["0 2 * * *"],
      origin,
    };
    render(pane());
    expect(screen.getByText(/Deployment schedule:/)).toHaveTextContent(
      `Deployment schedule: Syncs every day at 02:00 UTC. (0 2 * * * UTC, from ${source}), overridden by this request`
    );
  }
);

it("hides the deployment override line when no request exists", () => {
  mocks.query.data!.schedule.inherited = {
    crons: ["0 2 * * *"],
    origin: "annotation",
  };
  mocks.query.data!.schedule.requested = null;
  render(pane());
  expect(screen.queryByText(/Deployment schedule:/)).not.toBeInTheDocument();
});

it.each([true, false])(
  "names the reset destination when recorded: %s",
  async (known) => {
    mocks.query.data!.schedule.inherited = known
      ? { crons: ["0 2 * * *"], origin: "descriptor" }
      : null;
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(
      screen.getByRole("button", {
        name: known
          ? "Reset to the deployment schedule (Syncs every day at 02:00 UTC.)"
          : "Reset to the default schedule",
      })
    ).toBeInTheDocument();
  }
);
