vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/alerts-client");

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as alertsClient from "@/api/alerts-client";
import type { AlertPage, AlertSummary } from "@/api/alerts-types";
import { CustomApiError } from "@/api/custom-client";
import { ALERT, SUMMARY, wrapper } from "@/test/alerts";
import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { AlertsList } from "./alerts-list";

function page(
  alerts: AlertSummary[],
  { total = alerts.length, offset = 0 } = {}
): AlertPage {
  return { alerts, total, limit: 50, offset };
}

/** A read that never lands, so only what a write put in the cache can show. */
function neverLands<T>(): Promise<T> {
  return new Promise<T>(() => {});
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/alerts");
  scrollEndOutOfView();
});

describe("AlertsList", () => {
  it("lists each alert with its metric and links to both", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue(page([SUMMARY]));

    render(<AlertsList />, { wrapper });

    expect(
      await screen.findByRole("link", { name: "Too many open PRs" })
    ).toHaveAttribute("href", "/portal/custom/alerts/a1");
    expect(screen.getByRole("link", { name: "prs-open" })).toHaveAttribute(
      "href",
      "/portal/custom/metrics/prs-open"
    );
    expect(screen.getByRole("button", { name: "New alert" })).toHaveAttribute(
      "href",
      "/portal/custom/alerts/new"
    );
    expect(screen.getByText("1 alert")).toBeInTheDocument();
  });

  it("reads the next page when the end of the list comes into view", async () => {
    const first = Array.from({ length: 50 }, (_, at) => ({
      ...SUMMARY,
      id: `a${at}`,
      name: `Alert ${at}`,
    }));
    vi.mocked(alertsClient.fetchAlerts)
      .mockResolvedValueOnce(page(first, { total: 51 }))
      .mockResolvedValueOnce(
        page([{ ...SUMMARY, id: "a50", name: "Alert 50" }], {
          total: 51,
          offset: 50,
        })
      );

    render(<AlertsList />, { wrapper });
    expect(
      await screen.findByRole("link", { name: "Alert 0" })
    ).toBeInTheDocument();
    expect(screen.getByText("51 alerts")).toBeInTheDocument();

    scrollEndIntoView();

    expect(
      await screen.findByRole("link", { name: "Alert 50" })
    ).toBeInTheDocument();
    expect(alertsClient.fetchAlerts).toHaveBeenLastCalledWith(
      expect.objectContaining({ offset: 50 })
    );
  });

  it("turns checks off at the revision the alert is at now", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue(page([SUMMARY]));
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      revision: 7,
    });
    vi.mocked(alertsClient.setAlertEnabled).mockResolvedValue({
      ...ALERT,
      enabled: false,
      revision: 8,
    });

    render(<AlertsList />, { wrapper });
    await userEvent.click(
      await screen.findByRole("switch", {
        name: "Enable Too many open PRs",
      })
    );

    await waitFor(() =>
      expect(alertsClient.setAlertEnabled).toHaveBeenCalledWith("a1", false, 7)
    );
  });

  it("keeps a toggled row at its new state before the list is read again", async () => {
    vi.mocked(alertsClient.fetchAlerts)
      .mockResolvedValueOnce(page([SUMMARY]))
      .mockReturnValue(neverLands<AlertPage>());
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.setAlertEnabled).mockResolvedValue({
      ...ALERT,
      enabled: false,
      revision: 4,
    });

    render(<AlertsList />, { wrapper });
    const toggle = await screen.findByRole("switch", {
      name: "Enable Too many open PRs",
    });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);

    await waitFor(() => {
      expect(toggle).not.toHaveAttribute("aria-busy");
      expect(toggle).not.toBeChecked();
    });
    expect(alertsClient.setAlertEnabled).toHaveBeenCalledTimes(1);
    expect(alertsClient.fetchAlerts).toHaveBeenCalledTimes(2);
  });

  it("shows a refused toggle on its row", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue(page([SUMMARY]));
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
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

    render(<AlertsList />, { wrapper });
    await userEvent.click(
      await screen.findByRole("switch", {
        name: "Enable Too many open PRs",
      })
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Changed elsewhere. Try again."
    );
  });

  it("says what an alert is for when there are none", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue(page([]));

    render(<AlertsList />, { wrapper });

    expect(await screen.findByText("No alerts yet")).toBeInTheDocument();
  });

  it("shows the service's refusal when alerts are off", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Failed precondition",
        context: {
          violations: [
            {
              type: "disabled",
              subject: "alerts",
              description: "alerts are not enabled on this installation",
            },
          ],
        },
      })
    );

    render(<AlertsList />, { wrapper });

    expect(
      await screen.findByText("alerts are not enabled on this installation")
    ).toBeInTheDocument();
  });

  it("reads the list again when asked to retry", async () => {
    vi.mocked(alertsClient.fetchAlerts)
      .mockRejectedValueOnce(new CustomApiError(500, null))
      .mockResolvedValueOnce(page([SUMMARY]));

    render(<AlertsList />, { wrapper });
    expect(
      await screen.findByRole("status", { name: "Couldn't load the alerts." })
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));

    expect(
      await screen.findByRole("link", { name: "Too many open PRs" })
    ).toBeInTheDocument();
    expect(alertsClient.fetchAlerts).toHaveBeenCalledTimes(2);
  });

  it("searches by what was typed", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue(page([]));

    render(<AlertsList />, { wrapper });
    await userEvent.type(
      screen.getByRole("searchbox", { name: "Search alerts" }),
      "prs"
    );

    await waitFor(
      () =>
        expect(alertsClient.fetchAlerts).toHaveBeenCalledWith(
          expect.objectContaining({ search: "prs" })
        ),
      { timeout: 2_000 }
    );
    expect(await screen.findByText("No matching alerts")).toBeInTheDocument();
  });
});
