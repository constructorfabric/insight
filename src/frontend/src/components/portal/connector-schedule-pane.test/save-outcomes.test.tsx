import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";

import { ConnectorsApiError } from "@/api/connectors-client";
import { editCustom, mocks, pane, read } from "./fixtures";

it("does not announce Applied when a pending revision rolls back", () => {
  const { rerender } = render(pane());
  const restored = read();
  restored.schedule.revision = 1;
  restored.schedule.status = "applied";
  mocks.query.data = restored;
  rerender(pane());
  expect(mocks.toast).not.toHaveBeenCalled();
  expect(screen.getByText("Applied")).toBeInTheDocument();
});

it("does not announce Applied after a refused save reloads the same revision", async () => {
  const reason = "The schedule was not changed: synthetic refusal";
  const restored = read();
  restored.schedule.status = "applied";
  mocks.query.refetch.mockImplementation(async () => {
    mocks.query.data = restored;
    return { data: restored };
  });
  mocks.save.mockRejectedValue(new ConnectorsApiError(503, { detail: reason }));
  const { rerender } = render(pane());
  await editCustom("0 7 * * *");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(reason);
  rerender(pane());
  await waitFor(() => expect(mocks.query.refetch).toHaveBeenCalled());
  expect(mocks.toast).not.toHaveBeenCalled();
});

it("announces a newer revision becoming Applied after a failed save", async () => {
  mocks.query.refetch.mockResolvedValue({ data: read() });
  mocks.save.mockRejectedValue(
    new ConnectorsApiError(503, {
      detail: "The schedule was not changed: synthetic refusal",
    })
  );
  const { rerender } = render(pane());
  await editCustom();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByRole("alert");
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  const newer = read();
  newer.schedule.revision = 3;
  mocks.query.data = newer;
  rerender(pane());
  mocks.toast.mockClear();
  mocks.query.data = {
    ...newer,
    schedule: { ...newer.schedule, status: "applied" },
  };
  rerender(pane());
  await waitFor(() =>
    expect(mocks.toast).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "success",
        title: expect.stringMatching(/^Schedule applied/),
      })
    )
  );
});

it("shows a neutral loading state until the schedule arrives", () => {
  mocks.query.data = undefined;
  mocks.query.isPending = true;
  const { rerender } = render(pane());
  expect(screen.getByText("Loading…")).toBeInTheDocument();
  expect(screen.queryByText("Unavailable")).not.toBeInTheDocument();
  mocks.query.data = read();
  mocks.query.isPending = false;
  rerender(pane());
  expect(screen.queryByText("Loading…")).not.toBeInTheDocument();
  expect(screen.getByText("Waiting for reconcile")).toBeInTheDocument();
});

it.each([
  ["the instance is busy; try again", "Schedule not changed. Try again."],
  [
    "the cluster refused the change: synthetic private server detail",
    "Schedule not changed: the cluster refused the change.",
  ],
  [
    "the cluster did not answer in time: synthetic timeout detail",
    "Schedule not changed: the cluster did not answer in time.",
  ],
  [
    "the cluster returned an error: synthetic server detail",
    "Schedule not changed: the cluster returned an error.",
  ],
])(
  "keeps a refused save concise with its full reason in Details: %s",
  async (reason, summary) => {
    const detail = `The schedule was not changed: ${reason}`;
    mocks.query.refetch.mockResolvedValue({ data: read() });
    mocks.save.mockRejectedValue(new ConnectorsApiError(503, { detail }));
    render(pane());
    await editCustom();
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(summary);
    const details = alert.querySelector("details");
    expect(details).not.toHaveAttribute("open");
    expect(screen.getByText(detail)).not.toBeVisible();
    await userEvent.click(screen.getByText("Details"));
    expect(details).toHaveAttribute("open");
    expect(screen.getByText(detail)).toBeVisible();
  }
);

it("keeps a failed restoration concise and puts the cluster reason in Details", async () => {
  const detail =
    "The request is saved but not applied: the cluster refused the change: synthetic server detail; restoring the previous schedule failed";
  mocks.query.refetch.mockResolvedValue({ data: read() });
  mocks.save.mockRejectedValue(new ConnectorsApiError(503, { detail }));
  render(pane());
  await editCustom();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(
    "Request saved but not applied; restoring the previous schedule failed."
  );
  const details = alert.querySelector("details");
  expect(details).not.toHaveAttribute("open");
  expect(screen.getByText(detail)).not.toBeVisible();
  await userEvent.click(screen.getByText("Details"));
  expect(details).toHaveAttribute("open");
  expect(screen.getByText(detail)).toBeVisible();
});

it.each([409, 503])(
  "keeps the reconcile instruction visible when Reset loses its fallback: %s",
  async (status) => {
    const reason =
      "Reset requires the deployment schedule annotation; run reconcile before resetting";
    const detail =
      status === 503 ? `The schedule was not changed: ${reason}` : reason;
    mocks.query.refetch.mockResolvedValue({ data: read() });
    mocks.save.mockRejectedValue(new ConnectorsApiError(status, { detail }));
    render(pane());
    await editCustom();
    await userEvent.click(screen.getByRole("button", { name: /Reset to/ }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "Schedule not changed: run reconcile before resetting."
    );
    expect(alert.querySelector("details")).not.toHaveAttribute("open");
    expect(screen.getByText(detail)).not.toBeVisible();
    expect(alert.textContent?.split("Details")[0]).not.toContain("Try again");
  }
);

it.each([true, false])(
  "shows Paused without next runs with a saved override: %s",
  (override) => {
    const schedule = mocks.query.data!.schedule;
    schedule.status = "paused";
    schedule.live!.suspended = true;
    if (!override) schedule.requested = null;
    render(pane());
    expect(screen.getByText("Paused")).toBeInTheDocument();
    expect(screen.queryByText("Next (UTC)")).not.toBeInTheDocument();
    expect(mocks.toast).not.toHaveBeenCalled();
  }
);
