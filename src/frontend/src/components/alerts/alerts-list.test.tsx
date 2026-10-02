vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/alerts-client");

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as alertsClient from "@/api/alerts-client";
import { CustomApiError } from "@/api/custom-client";
import { scrollEndOutOfView } from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { AlertsList } from "./alerts-list";
import { ALERT, wrapper } from "@/test/alerts";

const SUMMARY = {
  id: "a1",
  name: "Too many open PRs",
  metric: "prs-open",
  enabled: true,
};

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/alerts");
  scrollEndOutOfView();
});

describe("AlertsList", () => {
  it("lists each alert with its metric and links to both", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue({
      alerts: [SUMMARY],
      total: 1,
      limit: 50,
      offset: 0,
    });

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
    expect(screen.getByText("1 alerts")).toBeInTheDocument();
  });

  it("turns checks off at the revision the alert is at now", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue({
      alerts: [SUMMARY],
      total: 1,
      limit: 50,
      offset: 0,
    });
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
        name: "Checks for Too many open PRs",
      })
    );

    await waitFor(() =>
      expect(alertsClient.setAlertEnabled).toHaveBeenCalledWith("a1", false, 7)
    );
  });

  it("shows a refused toggle on its row", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue({
      alerts: [SUMMARY],
      total: 1,
      limit: 50,
      offset: 0,
    });
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
        name: "Checks for Too many open PRs",
      })
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "It changed elsewhere. Try again."
    );
  });

  it("says what an alert is for when there are none", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue({
      alerts: [],
      total: 0,
      limit: 50,
      offset: 0,
    });

    render(<AlertsList />, { wrapper });

    expect(await screen.findByText(/No alerts yet/)).toBeInTheDocument();
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

  it("searches by what was typed", async () => {
    vi.mocked(alertsClient.fetchAlerts).mockResolvedValue({
      alerts: [],
      total: 0,
      limit: 50,
      offset: 0,
    });

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
    expect(
      await screen.findByText("No alert matches that search.")
    ).toBeInTheDocument();
  });
});
