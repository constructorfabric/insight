import { mocks, pane, read } from "./fixtures";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { ConnectorsApiError } from "@/api/connectors-client";

it("shows UTC next chips and UTC-only exact attribution and badge tooltips", async () => {
  const schedule = mocks.query.data!.schedule;
  schedule.status = "applied";
  schedule.live!.crons = ["30 23 * * *"];
  schedule.requested!.updated_at = "2026-01-15T11:58:00Z";
  const { container } = render(pane());
  const times = screen
    .getByText("Next (UTC)")
    .parentElement!.querySelectorAll("time");
  expect(Array.from(times, (time) => time.textContent)).toEqual([
    "23:30",
    "Fri 23:30",
    "Sat 23:30",
  ]);
  expect(container.textContent).not.toMatch(/your time|01:30|13:58|14:00/);
  const user = userEvent.setup();
  for (const [trigger, exact] of [
    [times[0], "2026-01-15 23:30:00 UTC"],
    [screen.getByText("set by you · 2 min ago"), "2026-01-15 11:58:00 UTC"],
    [screen.getByText("Applied"), "2026-01-15 12:00:00 UTC"],
  ] as const) {
    await user.hover(trigger.closest("[tabindex]")!);
    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toHaveTextContent(exact);
    expect(tooltip).not.toHaveTextContent(/your time|01:30|13:58|14:00/);
    await user.unhover(trigger.closest("[tabindex]")!);
  }
});

it("keeps editor preview chips and their exact tooltip in UTC", async () => {
  mocks.query.data!.schedule.requested!.crons = ["30 23 * * *"];
  const { container } = render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  const times = screen
    .getByText("Next (UTC)")
    .parentElement!.querySelectorAll("time");
  expect(Array.from(times, (time) => time.textContent)).toEqual([
    "23:30",
    "Fri 23:30",
    "Sat 23:30",
  ]);
  expect(container.textContent).not.toMatch(/your time|01:30/);
  await userEvent.hover(times[0].closest("[tabindex]")!);
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "2026-01-15 23:30:00 UTC"
  );
  expect(screen.getByRole("tooltip")).not.toHaveTextContent(/your time|01:30/);
});

it("announces the next sync in UTC in both toast and live region", () => {
  const view = render(pane());
  const next = read();
  next.schedule.status = "applied";
  next.schedule.live!.crons = ["30 12 * * *"];
  mocks.query.data = next;
  view.rerender(pane());
  const message = "Schedule applied · next sync 12:30 UTC";
  expect(mocks.toast).toHaveBeenCalledExactlyOnceWith({
    type: "success",
    title: message,
  });
  expect(
    view.container.querySelector('[aria-live="polite"]')
  ).toHaveTextContent(message);
  expect(view.container.textContent).not.toMatch(/your time|14:30/);
});

it("shows reconcile hints and their exact badge tooltip in UTC", async () => {
  mocks.query.data!.schedule.pending_reason = "awaiting_reconcile";
  mocks.query.data!.schedule.next_reconcile_at = "2026-01-15T12:15:00Z";
  const { container } = render(pane());
  expect(
    screen.getByText("Next reconcile expected around 12:15 UTC")
  ).toBeInTheDocument();
  await userEvent.hover(
    screen.getByText("Waiting for reconcile").closest("[tabindex]")!
  );
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "Next reconcile: 2026-01-15 12:15:00 UTC"
  );
  expect(container.textContent).not.toMatch(/your time|14:15/);
  expect(screen.getByRole("tooltip")).not.toHaveTextContent(/your time|14:15/);
});

it("dates a conflict reload in UTC", async () => {
  mocks.query.data!.schedule.requested!.crons = ["0 * * * *"];
  const next = read();
  next.etag = '"3"';
  next.schedule.revision = 3;
  next.schedule.requested!.updated_at = "2026-01-15T12:29:00Z";
  mocks.query.refetch.mockResolvedValue({ data: next });
  mocks.save.mockRejectedValue(new ConnectorsApiError(412, null));
  const { container } = render(pane());
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByRole("spinbutton", { name: "Minute" }), {
    target: { value: "30" },
  });
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "You changed this schedule in another tab at 12:29 UTC."
  );
  expect(container.textContent).not.toMatch(/your time|14:29/);
});
