import { mocks, pane, read, editCustom } from "./fixtures";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { ConnectorsApiError } from "@/api/connectors-client";

it.each(["0 25 * * *", "0 6 31 FEB *", "0 6 L * *", "0 6 * *", " 0\t6 * * * "])(
  "blocks invalid or duplicate custom row %s before Save",
  async (cron) => {
    render(pane());
    await editCustom("0 6 * * *");
    await userEvent.click(
      screen.getByRole("button", { name: "+ Add expression" })
    );
    const input = screen.getByRole("textbox", { name: "Cron expression 2" });
    fireEvent.change(input, { target: { value: cron } });
    expect(input).toHaveAttribute("aria-invalid", "false");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    fireEvent.blur(input);
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(
      document.getElementById(input.getAttribute("aria-describedby")!)
    ).toHaveAttribute("role", "alert");
    expect(
      screen.getByRole("textbox", { name: "Cron expression 1" })
    ).toHaveAttribute("aria-invalid", "false");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(mocks.save).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: "0 8 * * *" } });
    expect(input).toHaveAttribute("aria-invalid", "false");
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  }
);

it.each(["pending"] as const)(
  "reloads the visible hourly minute and payload after a %s conflict",
  async (status) => {
    mocks.query.data!.schedule.requested!.crons = ["59 * * * *"];
    mocks.query.data!.schedule.status = status;
    const next = read();
    next.etag = '"3"';
    next.schedule.revision = 3;
    next.schedule.status = status;
    next.schedule.requested!.crons = ["45 * * * *"];
    mocks.query.refetch.mockResolvedValue({ data: next });
    mocks.save
      .mockRejectedValueOnce(new ConnectorsApiError(412, null))
      .mockResolvedValueOnce(next);
    render(pane());
    await userEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "Minute" }), {
      target: { value: "58" },
    });
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByText(/You changed this schedule in another tab/);
    const minute = screen.getByRole("spinbutton", { name: "Minute" });
    expect(minute).toHaveValue("45");
    expect(minute).toHaveAttribute("aria-valuenow", "45");
    expect(minute).toHaveAttribute("aria-invalid", "false");
    if (status === "pending") {
      expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
      fireEvent.change(minute, { target: { value: "46" } });
    }
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(mocks.save).toHaveBeenLastCalledWith({
      etag: '"3"',
      crons: ["46 * * * *"],
    });
  }
);
