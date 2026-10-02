vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/alerts-client");

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as alertsClient from "@/api/alerts-client";
import type { AlertNotification } from "@/api/alerts-types";
import { CustomApiError } from "@/api/custom-client";
import { ALERT, wrapper } from "@/test/alerts";
import { portalRouter } from "@/test/portal-router";

import { AlertPage } from "./alert-page";

const SENT: AlertNotification = {
  id: "n1",
  rule_revision: 3,
  metric: "prs-open",
  column: "total",
  operator: ">",
  threshold: 10,
  value: 12,
  evaluated_at: "2026-10-01T09:00:00+00:00",
  destination: "ops",
  status: "sent",
  attempts: 1,
  provider_receipt: "msg-7",
  created_at: "2026-10-01T09:00:00+00:00",
};

function page(notifications: AlertNotification[], offset = 0) {
  return { notifications, limit: 20, offset };
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/alerts/a1");
});

describe("AlertPage", () => {
  it("shows the rule, the latest check and what it owed", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      range: "P7D",
      state: {
        last_evaluated_at: "2026-10-01T09:00:00+00:00",
        last_outcome: "breach",
        last_value: 12,
        last_valid_breached: true,
        breached_since: "2026-10-01T09:00:00+00:00",
      },
    });
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(
      page([
        SENT,
        {
          ...SENT,
          id: "n0",
          status: "failed",
          attempts: 5,
          provider_receipt: null,
          last_error:
            "the provider rejected the message: answered 400 Bad Request",
        },
      ])
    );

    render(<AlertPage id="a1" />, { wrapper });

    expect(
      await screen.findByRole("heading", { name: "Too many open PRs" })
    ).toBeInTheDocument();
    expect(screen.getByText("above 10")).toBeInTheDocument();
    expect(screen.getByText("Last 7 days")).toBeInTheDocument();
    expect(screen.getByText("Every 5 minutes")).toBeInTheDocument();
    expect(screen.getByText("Condition met")).toBeInTheDocument();
    expect(
      await screen.findByText("1 attempt to ops · receipt msg-7")
    ).toBeInTheDocument();
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(
      screen.getByText(
        "the provider rejected the message: answered 400 Bad Request"
      )
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit" })).toHaveAttribute(
      "href",
      "/portal/custom/alerts/a1/edit"
    );
  });

  it("says why a check could not decide", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      state: {
        last_evaluated_at: "2026-10-01T09:00:00+00:00",
        last_outcome: "unknown",
        last_reason: "many_rows",
      },
    });
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));

    render(<AlertPage id="a1" />, { wrapper });

    expect(await screen.findByText("Unknown")).toBeInTheDocument();
    expect(screen.getByText("More than one row")).toBeInTheDocument();
    expect(await screen.findByText("No notifications yet")).toBeInTheDocument();
  });

  it("says when an alert has not been checked", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      enabled: false,
    });
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));

    render(<AlertPage id="a1" />, { wrapper });

    expect(await screen.findByText("Disabled")).toBeInTheDocument();
  });

  it("pages older notifications until a short page ends them", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    const full = Array.from({ length: 20 }, (_, at) => ({
      ...SENT,
      id: `n${at}`,
    }));
    vi.mocked(alertsClient.fetchAlertNotifications)
      .mockResolvedValueOnce(page(full))
      .mockResolvedValueOnce(page([{ ...SENT, id: "old" }], 20));

    render(<AlertPage id="a1" />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Show older" })
    );

    await waitFor(() =>
      expect(alertsClient.fetchAlertNotifications).toHaveBeenLastCalledWith(
        "a1",
        {
          limit: 20,
          offset: 20,
        }
      )
    );
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Show older" })).toBeNull()
    );
  });

  it("deletes in two clicks and goes back to the list", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));
    vi.mocked(alertsClient.deleteAlert).mockResolvedValue(undefined);

    render(<AlertPage id="a1" />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Delete" })
    );
    await userEvent.click(screen.getByRole("button", { name: "Delete alert" }));

    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/alerts",
      })
    );
    expect(alertsClient.deleteAlert).toHaveBeenCalledWith("a1");
  });

  it("toggles at the revision the page holds", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));
    vi.mocked(alertsClient.setAlertEnabled).mockResolvedValue({
      ...ALERT,
      enabled: false,
      revision: 4,
    });

    render(<AlertPage id="a1" />, { wrapper });
    const header = await screen.findByRole("banner");
    await userEvent.click(
      within(header).getByRole("switch", {
        name: "Enable Too many open PRs",
      })
    );

    await waitFor(() =>
      expect(alertsClient.setAlertEnabled).toHaveBeenCalledWith("a1", false, 3)
    );
    const [sent] = vi.mocked(alertsClient.setAlertEnabled).mock
      .invocationCallOrder;
    const readsBefore = vi
      .mocked(alertsClient.fetchAlert)
      .mock.invocationCallOrder.filter((order) => order < sent);
    expect(readsBefore).toHaveLength(1);
  });

  it("reads the alert again after a toggle it was too late for", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));
    vi.mocked(alertsClient.setAlertEnabled).mockRejectedValue(
      new CustomApiError(409, {
        context: {
          violations: [
            {
              type: "revision",
              subject: "expected_revision",
              description: "alert a1 is at revision 4, not 3",
            },
          ],
        },
      })
    );

    render(<AlertPage id="a1" />, { wrapper });
    const header = await screen.findByRole("banner");
    await userEvent.click(
      within(header).getByRole("switch", {
        name: "Enable Too many open PRs",
      })
    );

    expect(
      await screen.findByText("Changed elsewhere. Try again.")
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(alertsClient.fetchAlert).toHaveBeenCalledTimes(2)
    );
  });

  it("shows a notification once when paging shifted under it", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    const full = Array.from({ length: 20 }, (_, at) => ({
      ...SENT,
      id: `n${at}`,
      provider_receipt: `r${at}`,
    }));
    vi.mocked(alertsClient.fetchAlertNotifications)
      .mockResolvedValueOnce(page(full))
      .mockResolvedValueOnce(page([full[19]], 20));

    render(<AlertPage id="a1" />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Show older" })
    );

    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Show older" })).toBeNull()
    );
    expect(screen.getAllByText(/receipt r19$/)).toHaveLength(1);
  });

  it("names a missing alert and the way back", async () => {
    vi.mocked(alertsClient.fetchAlert).mockRejectedValue(
      new CustomApiError(404, { detail: "alert a1 was not found" })
    );

    render(<AlertPage id="a1" />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "alert a1 was not found"
    );
    expect(
      screen.getByRole("link", { name: "Back to alerts" })
    ).toHaveAttribute("href", "/portal/custom/alerts");
  });
});
