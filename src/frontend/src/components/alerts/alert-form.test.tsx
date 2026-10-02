vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/alerts-client");
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchMetricNames: vi.fn(),
    fetchMetric: vi.fn(),
    runMetric: vi.fn(),
  };
});

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as alertsClient from "@/api/alerts-client";
import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { ALERT, wrapper } from "@/test/alerts";
import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { EditAlertPage, NewAlertPage } from "./alert-form";

const PRS_OPEN = {
  definition: {
    table: "gold.prs",
    fields: [
      { column: "id", type: "int", agg: "count", as_name: "total" },
      { column: "repo", type: "string", as_name: "repo" },
    ],
  },
};

function answer(rows: unknown[][], columns = ["total"]) {
  return { columns, rows, percents: [] };
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/alerts/new");
  vi.mocked(alertsClient.fetchAlertDestinations).mockResolvedValue([
    { name: "ops", provider: "zulip" },
  ]);
  vi.mocked(customClient.fetchMetricNames).mockResolvedValue({
    names: ["prs-open"],
    total: 1,
  });
  vi.mocked(customClient.fetchMetric).mockResolvedValue(
    PRS_OPEN as unknown as Awaited<ReturnType<typeof customClient.fetchMetric>>
  );
  vi.mocked(customClient.runMetric).mockResolvedValue(
    answer([[12]]) as unknown as Awaited<
      ReturnType<typeof customClient.runMetric>
    >
  );
});

/** Opens a kit select by its label and picks one of its options. */
async function pick(label: string, option: string) {
  await userEvent.click(screen.getByRole("combobox", { name: label }));
  await userEvent.click(await screen.findByRole("option", { name: option }));
}

async function fillNew() {
  await userEvent.type(
    await screen.findByLabelText("Name"),
    "Too many open PRs"
  );
  await userEvent.type(screen.getByLabelText("Metric"), "prs");
  await userEvent.click(
    await screen.findByRole("option", { name: "prs-open" })
  );
  await pick("Column", "total");
  await userEvent.type(screen.getByLabelText("Threshold"), "10");
}

describe("NewAlertPage", () => {
  it("creates the rule the form says and opens it", async () => {
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith({
        name: "Too many open PRs",
        metric: "prs-open",
        column: "total",
        operator: ">",
        threshold: 10,
        range: null,
        interval_secs: 300,
        destination: "ops",
        enabled: true,
      })
    );
    expect(portalRouter.navigations).toContainEqual({
      to: "/portal/custom/alerts/$id",
      params: { id: "a1" },
    });
  });

  it("reads only the metric picked, not each one typed", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(customClient.fetchMetric).toHaveBeenCalledTimes(1);
    expect(customClient.fetchMetric).toHaveBeenCalledWith("prs-open");
  });

  it("asks the service for the metrics matching what is typed", async () => {
    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "gold.prs");

    await waitFor(() =>
      expect(customClient.fetchMetricNames).toHaveBeenCalledWith(
        expect.objectContaining({ search: "gold.prs" })
      )
    );
  });

  it("reads the next page of metrics when the list is scrolled to its end", async () => {
    scrollEndOutOfView();
    const first = Array.from({ length: 50 }, (_, at) => `metric-${at}`);
    vi.mocked(customClient.fetchMetricNames)
      .mockResolvedValueOnce({ names: first, total: 51 })
      .mockResolvedValueOnce({ names: ["metric-50"], total: 51 });

    render(<NewAlertPage />, { wrapper });
    await userEvent.click(await screen.findByLabelText("Metric"));
    expect(
      await screen.findByRole("option", { name: "metric-0" })
    ).toBeInTheDocument();

    scrollEndIntoView();

    expect(
      await screen.findByRole("option", { name: "metric-50" })
    ).toBeInTheDocument();
    expect(customClient.fetchMetricNames).toHaveBeenLastCalledWith(
      expect.objectContaining({ offset: 50 })
    );
  });

  it("says a grouped metric cannot be alerted on and offers only its numbers", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: {
        dataset: "runs",
        fields: [
          { field: "stand", type: "string", as_name: "stand" },
          { field: "runs", type: "int", agg: "sum", as_name: "runs" },
        ],
        group_by: ["stand"],
      },
    } as unknown as Awaited<ReturnType<typeof customClient.fetchMetric>>);

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );

    expect(
      await screen.findByText(/Returns one row per stand/)
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("combobox", { name: "Column" }));
    expect(
      await screen.findByRole("option", { name: "runs" })
    ).toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "stand" })).toBeNull();
  });

  it("explains an empty ratio as nothing to divide by", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: {
        dataset: "runs",
        fields: [
          { field: "passed", type: "int", agg: "sum", as_name: "passed" },
          { field: "runs", type: "int", agg: "sum", as_name: "runs" },
          { type: "float", as_name: "pass_rate", divide: ["passed", "runs"] },
        ],
      },
    } as unknown as Awaited<ReturnType<typeof customClient.fetchMetric>>);
    vi.mocked(customClient.runMetric).mockResolvedValue(
      answer(
        [[0, 0, null]],
        ["passed", "runs", "pass_rate"]
      ) as unknown as Awaited<ReturnType<typeof customClient.runMetric>>
    );

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );
    await pick("Column", "pass_rate");

    expect(
      await screen.findByText(/No data in this window/, {}, { timeout: 3_000 })
    ).toBeInTheDocument();
  });

  it("moves to the first field that needs fixing", async () => {
    render(<NewAlertPage />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Create alert" })
    );

    expect(screen.getByLabelText("Name")).toHaveFocus();
  });

  it("shows what a check would read now", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(
      await screen.findByText(/meets the condition/, {}, { timeout: 3_000 })
    ).toBeInTheDocument();
    expect(customClient.runMetric).toHaveBeenCalledWith("prs-open", {
      bucket: false,
    });
  });

  it("warns before saving a metric no check could read", async () => {
    vi.mocked(customClient.runMetric).mockResolvedValue(
      answer([[1], [2]]) as unknown as Awaited<
        ReturnType<typeof customClient.runMetric>
      >
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(
      await screen.findByText(
        /Current value: unknown · More than one row/,
        {},
        {
          timeout: 3_000,
        }
      )
    ).toBeInTheDocument();
  });

  it("says what is missing before it sends anything", async () => {
    render(<NewAlertPage />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Create alert" })
    );

    expect(await screen.findByText("Enter a name.")).toBeInTheDocument();
    expect(screen.getByText("Pick a metric.")).toBeInTheDocument();
    expect(screen.getByText("Enter a number.")).toBeInTheDocument();
    expect(alertsClient.createAlert).not.toHaveBeenCalled();
  });

  it("puts the service's refusal on the field it names", async () => {
    vi.mocked(alertsClient.createAlert).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Request validation failed",
        context: {
          field_violations: [
            {
              field: "interval_secs",
              description: "interval must be 60 to 604800 seconds",
              reason: "INVALID",
            },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    expect(
      await screen.findByText("interval must be 60 to 604800 seconds")
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Check every")).toHaveAttribute(
      "aria-invalid",
      "true"
    );
  });

  it("checks at a custom interval", async () => {
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await pick("Check every", "Custom…");
    const every = screen.getByLabelText("Amount");
    await userEvent.clear(every);
    await userEvent.type(every, "2");
    await pick("Unit", "hours");
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith(
        expect.objectContaining({ interval_secs: 7_200 })
      )
    );
  });

  it("says so when there is nowhere to send to", async () => {
    vi.mocked(alertsClient.fetchAlertDestinations).mockResolvedValue([]);

    render(<NewAlertPage />, { wrapper });

    expect(
      await screen.findByText(/No destinations are configured/)
    ).toBeInTheDocument();
  });
});

describe("EditAlertPage", () => {
  beforeEach(() => portalRouter.reset("/portal/custom/alerts/a1/edit"));

  it("replaces the alert at the revision it was opened at", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.replaceAlert).mockResolvedValue({
      ...ALERT,
      threshold: 20,
      revision: 4,
    });

    render(<EditAlertPage id="a1" />, { wrapper });
    const threshold = await screen.findByLabelText("Threshold");
    expect(threshold).toHaveValue("10");
    await userEvent.clear(threshold);
    await userEvent.type(threshold, "20");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(alertsClient.replaceAlert).toHaveBeenCalledWith("a1", {
        name: "Too many open PRs",
        metric: "prs-open",
        column: "total",
        operator: ">",
        threshold: 20,
        range: null,
        interval_secs: 300,
        destination: "ops",
        enabled: true,
        expected_revision: 3,
      })
    );
    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/alerts/$id",
        params: { id: "a1" },
      })
    );
  });

  it("sends a stored interval back unchanged when only the name changed", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      interval_secs: 90,
    });
    vi.mocked(alertsClient.replaceAlert).mockResolvedValue({
      ...ALERT,
      revision: 4,
    });

    render(<EditAlertPage id="a1" />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Name"), "!");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(alertsClient.replaceAlert).toHaveBeenCalledWith(
        "a1",
        expect.objectContaining({ interval_secs: 90 })
      )
    );
  });

  it("keeps the edits when the alert moved on, and offers its latest version", async () => {
    vi.mocked(alertsClient.fetchAlert)
      .mockResolvedValueOnce(ALERT)
      .mockResolvedValue({ ...ALERT, threshold: 15, revision: 4 });
    vi.mocked(alertsClient.replaceAlert).mockRejectedValue(
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

    render(<EditAlertPage id="a1" />, { wrapper });
    const threshold = await screen.findByLabelText("Threshold");
    await userEvent.clear(threshold);
    await userEvent.type(threshold, "20");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(
      await screen.findByText(/changed by someone else/)
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Threshold")).toHaveValue("20");

    await userEvent.click(
      screen.getByRole("button", { name: "Load latest version" })
    );
    await waitFor(() =>
      expect(screen.getByLabelText("Threshold")).toHaveValue("15")
    );
  });
});
