vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/alerts-client");

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as alertsClient from "@/api/alerts-client";
import type { Alert, AlertNotification } from "@/api/alerts-types";
import { CustomApiError } from "@/api/custom-client";
import { ALERT, NOTIFICATION, wrapper } from "@/test/alerts";
import { portalRouter } from "@/test/portal-router";

import { AlertPage } from "./alert-page";

const OFF_HINT = "Turning it off withdraws notifications not yet sent.";

function page(notifications: AlertNotification[], offset = 0) {
  return { notifications, limit: 20, offset };
}

/** A read that never lands, so only what a write put in the cache can show. */
function neverLands<T>(): Promise<T> {
  return new Promise<T>(() => {});
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
        NOTIFICATION,
        {
          ...NOTIFICATION,
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
    expect(screen.getByText("Last 7 days")).toBeInTheDocument();
    expect(screen.getByText("Check every")).toBeInTheDocument();
    expect(screen.getByText("5 minutes")).toBeInTheDocument();
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

  it("reads each notification against the condition it was owed under", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(
      page([
        NOTIFICATION,
        {
          ...NOTIFICATION,
          id: "n0",
          rule_revision: 2,
          column: "count",
          threshold: 5,
          value: 7,
        },
      ])
    );

    render(<AlertPage id="a1" />, { wrapper });
    const table = within(await screen.findByRole("table"));

    expect(table.getByText("above 10")).toBeInTheDocument();
    expect(table.getByText("above 5")).toBeInTheDocument();
    expect(table.getByText("prs-open · count")).toBeInTheDocument();
    expect(table.queryByText("prs-open · total")).toBeNull();
  });

  it("names the zone the check times are shown in", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(
      page([NOTIFICATION])
    );

    render(<AlertPage id="a1" />, { wrapper });
    const table = within(await screen.findByRole("table"));

    expect(table.getByText("1 Oct 2026")).toBeInTheDocument();
    expect(table.getByText("23:00 GMT+14")).toBeInTheDocument();
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
    expect(screen.queryByText(OFF_HINT)).toBeNull();
  });

  it("warns beside the switch what turning it off withdraws", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));

    render(<AlertPage id="a1" />, { wrapper });

    expect(
      await screen.findByRole("switch", { name: "Enable Too many open PRs" })
    ).toHaveAccessibleDescription(OFF_HINT);
  });

  it("pages older notifications until a short page ends them", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    const full = Array.from({ length: 20 }, (_, at) => ({
      ...NOTIFICATION,
      id: `n${at}`,
    }));
    vi.mocked(alertsClient.fetchAlertNotifications)
      .mockResolvedValueOnce(page(full))
      .mockResolvedValueOnce(page([{ ...NOTIFICATION, id: "old" }], 20));

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
    await userEvent.click(
      screen.getByRole("button", { name: "Delete it, with its notifications" })
    );

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

  it("shows the new state while the change is saved, and keeps it once saved", async () => {
    vi.mocked(alertsClient.fetchAlert)
      .mockResolvedValueOnce(ALERT)
      .mockReturnValue(neverLands<Alert>());
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));
    let saved!: (alert: Alert) => void;
    vi.mocked(alertsClient.setAlertEnabled).mockReturnValue(
      new Promise<Alert>((resolve) => {
        saved = resolve;
      })
    );

    render(<AlertPage id="a1" />, { wrapper });
    const toggle = await screen.findByRole("switch", {
      name: "Enable Too many open PRs",
    });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);

    await waitFor(() => expect(toggle).toHaveAttribute("aria-busy", "true"));
    expect(toggle).not.toBeChecked();
    expect(screen.getByText("Off")).toBeInTheDocument();

    saved({ ...ALERT, enabled: false, revision: 4 });

    await waitFor(() => expect(toggle).not.toHaveAttribute("aria-busy"));
    expect(toggle).not.toBeChecked();
    expect(alertsClient.fetchAlert).toHaveBeenCalledTimes(2);
  });

  it("puts the switch back and says so when the change is refused", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.fetchAlertNotifications).mockResolvedValue(page([]));
    vi.mocked(alertsClient.setAlertEnabled).mockRejectedValue(
      new CustomApiError(500, null)
    );

    render(<AlertPage id="a1" />, { wrapper });
    const toggle = await screen.findByRole("switch", {
      name: "Enable Too many open PRs",
    });
    await userEvent.click(toggle);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't change it."
    );
    expect(toggle).toBeChecked();
    expect(toggle).not.toHaveAttribute("aria-busy");
    expect(alertsClient.fetchAlert).toHaveBeenCalledTimes(1);
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
      ...NOTIFICATION,
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
