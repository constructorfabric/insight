import { pickPreset } from "./preset";
import { mocks, pane, read } from "./fixtures";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

it.each([
  [
    "applied",
    null,
    "Applied",
    "text-success",
    "lucide-check",
    "Request revision 2 is running",
  ],
  [
    "pending",
    "different_schedule",
    "Waiting for reconcile",
    "text-warning",
    "lucide-clock",
    "The sync job runs a different schedule",
  ],
  [
    "pending",
    "awaiting_reconcile",
    "Waiting for reconcile",
    "text-warning",
    "lucide-clock",
    "The sync job is not yet prepared",
  ],
  [
    "pending",
    "no_cronworkflow",
    "Not scheduled yet",
    "text-warning",
    "lucide-clock",
    "This instance has no sync schedule yet",
  ],
  [
    "pending",
    null,
    "Waiting for reconcile",
    "text-warning",
    "lucide-clock",
    "Runs now, checked at",
  ],
  [
    "inherited",
    null,
    "Default",
    "text-muted-foreground",
    "lucide-minus",
    "connector descriptor",
  ],
] as const)(
  "uses one %s %s badge with an icon and details only in its tooltip",
  async (status, reason, label, tone, icon, detail) => {
    const schedule = mocks.query.data!.schedule;
    schedule.status = status;
    schedule.pending_reason = reason;
    schedule.inherited = { crons: ["0 2 * * *"], origin: "descriptor" };

    if (status === "inherited") schedule.requested = null;
    const { container } = render(pane());
    const badge = screen.getByText(label).closest("[title]")!;
    expect(
      container.querySelectorAll('[data-slot="schedule-header-status"]')
    ).toHaveLength(1);
    expect(badge).toHaveClass(tone);
    expect(badge.querySelector(`.${icon}`)).toHaveAttribute(
      "aria-hidden",
      "true"
    );
    expect(screen.queryByRole("list")).toBe(
      status === "applied" || status === "inherited"
        ? screen.getByRole("list")
        : null
    );
    expect(container.textContent).not.toMatch(
      /First sync|✓ Saved|Saved and applied at|Runs the deployment schedule/i
    );
    expect(screen.queryByText("Synthetic fixed error")).not.toBeInTheDocument();
    if (status === "applied" || status === "inherited")
      expect(container.querySelector('[data-slot="schedule-hint"]')).toBeNull();
    await userEvent.hover(badge.closest("[tabindex]")!);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(detail);
  }
);

it.each([
  [
    "pending",
    "different_schedule",
    false,
    "Takes effect at the next reconcile pass",
  ],
  [
    "pending",
    "awaiting_reconcile",
    true,
    "Next reconcile expected around 12:15 UTC",
  ],
  [
    "pending",
    "no_cronworkflow",
    false,
    "Takes effect at the next reconcile pass",
  ],
  ["pending", null, false, "Takes effect at the next reconcile pass"],
] as const)("shows a single hint for %s %s", (status, reason, known, hint) => {
  const schedule = mocks.query.data!.schedule;
  schedule.status = status;
  schedule.pending_reason = reason;
  schedule.next_reconcile_at = known ? "2026-01-15T12:15:00Z" : null;
  const { container } = render(pane());
  expect(
    container.querySelectorAll('[data-slot="schedule-hint"]')
  ).toHaveLength(1);
  expect(screen.getByText(new RegExp(hint))).toBeInTheDocument();
  expect(screen.getByText(/until then runs/)).toHaveTextContent(
    "every day at 03:00"
  );
});

it.each(["applied"] as const)(
  "toasts and announces a transition to %s once, including a new revision",
  (status) => {
    const { rerender, container } = render(pane());
    expect(mocks.toast).not.toHaveBeenCalled();
    expect(
      container.querySelector('[aria-live="polite"] > span')
    ).toBeEmptyDOMElement();
    const next = read();
    next.schedule.status = status;
    next.schedule.live!.crons = ["*/5 * * * *"];
    const message = "Schedule applied · next sync 12:05 UTC";
    mocks.query.data = next;
    rerender(pane());
    expect(mocks.toast).toHaveBeenCalledExactlyOnceWith({
      type: "success",
      title: message,
    });
    expect(
      container.querySelector('[aria-live="polite"] > span')
    ).toHaveTextContent(message);
    mocks.query.data = structuredClone(next);
    rerender(pane());
    expect(mocks.toast).toHaveBeenCalledTimes(1);
    mocks.query.data!.schedule.revision++;
    rerender(pane());
    expect(mocks.toast).toHaveBeenCalledTimes(2);
  }
);

it("does not toast initial applied data, and announces immediate saves while the editor is mounted", async () => {
  mocks.query.data!.schedule.status = "applied";
  const view = render(pane());
  expect(mocks.toast).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  mocks.save.mockImplementation(async () => {
    const next = read();
    next.schedule.status = "applied";
    next.schedule.revision = 3;
    mocks.query.data = next;
    view.rerender(pane());
    return next;
  });
  await pickPreset("hours-1");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(mocks.toast).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Edit" })).toHaveFocus();
});

it("puts exact dates behind the short UTC Next chips and relative attribution", async () => {
  mocks.query.data!.schedule.status = "applied";
  mocks.query.data!.schedule.live!.crons = ["*/5 * * * *"];
  mocks.query.data!.schedule.requested!.updated_at = "2026-01-15T11:59:20Z";
  render(pane());
  expect(screen.getByText("set by you · just now")).toHaveClass(
    "text-muted-foreground"
  );
  const preview = screen.getByText("Next (UTC)").parentElement!;
  expect(within(preview).getAllByRole("listitem")).toHaveLength(3);
  const first = preview.querySelector("time")!;
  expect(first).toHaveTextContent("12:05");
  expect(first).not.toHaveTextContent(/UTC|2026|in 5/);
  await userEvent.hover(first.closest("[tabindex]")!);
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "2026-01-15 12:05:00 UTC"
  );
  expect(screen.getByRole("tooltip")).not.toHaveTextContent("your time");
});

it("keeps drift waiting without a toast and reports unreadable jobs", () => {
  const { rerender } = render(pane());
  const schedule = mocks.query.data!.schedule;

  schedule.pending_reason = "different_schedule";
  schedule.revision++;
  rerender(pane());
  expect(screen.getByText("Waiting for reconcile")).toBeInTheDocument();
  expect(mocks.toast).not.toHaveBeenCalled();
  schedule.status = null;
  schedule.read_error = "The live sync job cannot be read.";
  rerender(pane());
  expect(screen.getByText("Unavailable")).toBeInTheDocument();
  expect(screen.getByText(schedule.read_error)).toBeInTheDocument();
  expect(screen.getByText("0 6 * * *")).toBeInTheDocument();
  expect(mocks.toast).not.toHaveBeenCalled();
});
