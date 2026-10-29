vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/alerts-client");
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchMetrics: vi.fn(),
    fetchMetric: vi.fn(),
    fetchDataset: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as alertsClient from "@/api/alerts-client";
import type { Alert } from "@/api/alerts-client";
import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { alertQuery } from "@/queries/alerts";
import { ALERT, wrapper } from "@/test/alerts";
import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";
import { portalRouter } from "@/test/portal-router";

import { EditAlertPage, NewAlertPage } from "./alert-form";

function listed(names: string[]) {
  return names.map((name) => ({ id: `id-${name}`, name }));
}

const PRS_OPEN = {
  name: "prs-open",
  definition: {
    table: "gold.prs",
    fields: [{ column: "id", type: "int", agg: "count", as_name: "total" }],
  },
};

const ABOVE_TEN = {
  rows: "one",
  where: { column: "total", op: ">", value: 10 },
};

/** What the service answers when the one row meets the condition. */
const MET = {
  outcome: "breach" as const,
  reason: null,
  witness: { matched: 1, answered: 1, columns: ["total"], rows: [[12]] },
};

/** The same metric with a date, so it can be read over a window. */
const DATED_PRS_OPEN = {
  ...PRS_OPEN,
  clock: { field: "created_at", from: "metric" },
};

/** What the service says when the alert moved on since the form was opened. */
const MOVED_ON = new CustomApiError(409, {
  context: {
    violations: [
      {
        type: "revision",
        subject: "expected_revision",
        description: "alert a1 is at revision 4, not 3",
      },
    ],
  },
});

function storedMetric(metric: unknown) {
  return metric as Awaited<ReturnType<typeof customClient.fetchMetric>>;
}

/** A client holding what the alert page cached before Edit was clicked. */
function cachedWrapper(stale: Alert) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  queryClient.setQueryData(alertQuery(stale.id).queryKey, stale);

  return function Cached({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

afterEach(() => {
  vi.useRealTimers();
});

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/alerts/new");
  vi.mocked(alertsClient.fetchAlertDestinations).mockResolvedValue([
    { name: "ops", provider: "zulip" },
  ]);
  vi.mocked(customClient.fetchMetrics).mockResolvedValue({
    items: listed(["prs-open"]),
    total: 1,
  });
  vi.mocked(customClient.fetchMetric).mockResolvedValue(storedMetric(PRS_OPEN));
  vi.mocked(alertsClient.previewAlert).mockResolvedValue(MET);
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
  await pickColumn(1, "total");
  await userEvent.type(condition(1).getByLabelText("Value"), "10");
}

function condition(number: number) {
  return within(screen.getByRole("group", { name: `Condition ${number}` }));
}

/** Picks a test's column once the metric's columns are offered. */
async function pickColumn(number: number, column: string) {
  await userEvent.click(
    condition(number).getByRole("combobox", { name: "Column" })
  );
  await userEvent.click(await screen.findByRole("option", { name: column }));
}

/** The JSON view's text, read as the document it shows. */
function shownJson(): Record<string, unknown> {
  return JSON.parse(
    (screen.getByLabelText("Alert as JSON") as HTMLTextAreaElement).value
  ) as Record<string, unknown>;
}

/** Replaces the JSON view's text in one change, as a paste would. */
function typeJson(text: string) {
  fireEvent.change(screen.getByLabelText("Alert as JSON"), {
    target: { value: text },
  });
}

/** An alert whose condition reads `value` against its threshold. */
function alertAt(value: unknown, change: Partial<Alert> = {}): Alert {
  return {
    ...ALERT,
    condition: {
      rows: "one",
      where: { column: "total", op: ">", value },
    } as Alert["condition"],
    ...change,
  };
}

// WORKAROUND: each case fills the whole form, which overruns the 5 s default on loaded CI runners.
describe("NewAlertPage", { timeout: 20_000 }, () => {
  it("creates the rule the form says and opens it", async () => {
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith({
        name: "Too many open PRs",
        metric: "id-prs-open",
        condition: ABOVE_TEN,
        context_columns: [],
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

  it.each([
    ["Last 30 days", "P30D"],
    ["All time", null],
  ])(
    "sends the condition, destination and switch as set, with the window %s",
    async (window, range) => {
      vi.mocked(alertsClient.fetchAlertDestinations).mockResolvedValue([
        { name: "ops", provider: "zulip" },
        { name: "dev", provider: "zulip" },
      ]);
      vi.mocked(customClient.fetchMetric).mockResolvedValue(
        storedMetric(DATED_PRS_OPEN)
      );
      vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

      render(<NewAlertPage />, { wrapper });
      await fillNew();
      await pick("Operator", "below");
      for (const [label, option] of [
        ["Window", window],
        ["Destination", "dev (zulip)"],
      ]) {
        await pick(label, option);
      }
      await userEvent.click(screen.getByRole("switch"));
      await userEvent.click(
        screen.getByRole("button", { name: "Create alert" })
      );

      await waitFor(() =>
        expect(alertsClient.createAlert).toHaveBeenCalledWith({
          name: "Too many open PRs",
          metric: "id-prs-open",
          condition: {
            rows: "one",
            where: { column: "total", op: "<", value: 10 },
          },
          context_columns: [],
          range,
          interval_secs: 300,
          destination: "dev",
          enabled: false,
        })
      );
    }
  );

  it("reads only the metric picked, not each one typed", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(customClient.fetchMetric).toHaveBeenCalledTimes(1);
    expect(customClient.fetchMetric).toHaveBeenCalledWith("id-prs-open");
  });

  it("says when the picked metric cannot be read, and reads it again on request", async () => {
    vi.mocked(customClient.fetchMetric)
      .mockRejectedValueOnce(new CustomApiError(500, null))
      .mockResolvedValueOnce(storedMetric(PRS_OPEN));

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );

    expect(
      await screen.findByText("Couldn't read the metric.")
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));

    await pickColumn(1, "total");
    expect(
      screen.queryByText("Couldn't read the metric.")
    ).not.toBeInTheDocument();
    expect(customClient.fetchMetric).toHaveBeenCalledTimes(2);
  });

  it("offers each metric by its name and stores the one picked by its id", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [{ id: "m-1", name: "Lines per day" }],
      total: 1,
    });
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Name"), "Few lines");
    await userEvent.type(screen.getByLabelText("Metric"), "lines");
    await userEvent.click(
      await screen.findByRole("option", { name: "Lines per day" })
    );
    await pickColumn(1, "total");
    await userEvent.type(condition(1).getByLabelText("Value"), "10");

    expect(customClient.fetchMetric).toHaveBeenCalledWith("m-1");
    expect(screen.getByLabelText("Metric")).toHaveValue("Lines per day");

    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));
    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith(
        expect.objectContaining({ metric: "m-1" })
      )
    );
  });

  it("asks the service for the metrics matching what is typed", async () => {
    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "gold.prs");

    await waitFor(() =>
      expect(customClient.fetchMetrics).toHaveBeenCalledWith(
        expect.objectContaining({ search: "gold.prs" })
      )
    );
  });

  it("reads the next page of metrics when the list is scrolled to its end", async () => {
    scrollEndOutOfView();
    const first = Array.from({ length: 50 }, (_, at) => `metric-${at}`);
    vi.mocked(customClient.fetchMetrics)
      .mockResolvedValueOnce({ items: listed(first), total: 51 })
      .mockResolvedValueOnce({ items: listed(["metric-50"]), total: 51 });

    render(<NewAlertPage />, { wrapper });
    await userEvent.click(await screen.findByLabelText("Metric"));
    expect(
      await screen.findByRole("option", { name: "metric-0" })
    ).toBeInTheDocument();

    scrollEndIntoView();

    expect(
      await screen.findByRole("option", { name: "metric-50" })
    ).toBeInTheDocument();
    expect(customClient.fetchMetrics).toHaveBeenLastCalledWith(
      expect.objectContaining({ offset: 50 })
    );
  });

  it("says when the next page of metrics cannot be read, and reads it again on request", async () => {
    scrollEndOutOfView();
    const first = Array.from({ length: 50 }, (_, at) => `metric-${at}`);
    vi.mocked(customClient.fetchMetrics)
      .mockResolvedValueOnce({ items: listed(first), total: 51 })
      .mockRejectedValueOnce(new CustomApiError(500, null))
      .mockResolvedValueOnce({ items: listed(["metric-50"]), total: 51 });

    render(<NewAlertPage />, { wrapper });
    await userEvent.click(await screen.findByLabelText("Metric"));
    await screen.findByRole("option", { name: "metric-0" });

    scrollEndIntoView();

    expect(
      await screen.findByText("Couldn't load more metrics.")
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "metric-0" })
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));

    expect(
      await screen.findByRole("option", { name: "metric-50" })
    ).toBeInTheDocument();
  });

  it("offers a metric once when a later page repeats it", async () => {
    scrollEndOutOfView();
    const first = Array.from({ length: 50 }, (_, at) => `metric-${at}`);
    vi.mocked(customClient.fetchMetrics)
      .mockResolvedValueOnce({ items: listed(first), total: 51 })
      .mockResolvedValueOnce({ items: listed(["metric-49", "metric-50"]), total: 52 });

    render(<NewAlertPage />, { wrapper });
    await userEvent.click(await screen.findByLabelText("Metric"));
    await screen.findByRole("option", { name: "metric-0" });

    scrollEndIntoView();

    await screen.findByRole("option", { name: "metric-50" });
    expect(screen.getAllByRole("option", { name: "metric-49" })).toHaveLength(
      1
    );
  });

  it("asks which rows must match on a grouped metric, and tests its group too", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(
      storedMetric({
        definition: {
          dataset: "runs",
          fields: [
            { field: "stand", type: "string", as_name: "stand" },
            { field: "runs", type: "int", agg: "sum", as_name: "runs" },
          ],
          group_by: ["stand"],
        },
      })
    );
    vi.mocked(customClient.fetchDataset).mockResolvedValue({
      name: "runs",
      declaration: {
        title: "Runs",
        fields: [{ name: "stand", path: "stand", type: "string" }],
      },
    } as unknown as Awaited<ReturnType<typeof customClient.fetchDataset>>);

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );
    await pickColumn(1, "stand");

    expect(await screen.findByLabelText("Rows")).toHaveTextContent(
      "Exactly one row"
    );
    await userEvent.click(
      condition(1).getByRole("combobox", { name: "Operator" })
    );
    expect(
      await screen.findByRole("option", { name: "is one of" })
    ).toBeInTheDocument();
  });

  it("explains an empty ratio as nothing to divide by", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(
      storedMetric({
        definition: {
          table: "gold.runs",
          fields: [
            { column: "passed", type: "int", agg: "sum", as_name: "passed" },
            { column: "runs", type: "int", agg: "sum", as_name: "runs" },
            { type: "float", as_name: "pass_rate", divide: ["passed", "runs"] },
          ],
        },
      })
    );
    vi.mocked(alertsClient.previewAlert).mockResolvedValue({
      outcome: "unknown",
      reason: "null",
      witness: null,
    });

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );
    await pickColumn(1, "pass_rate");
    await userEvent.type(condition(1).getByLabelText("Value"), "0.5");

    expect(
      await screen.findByText(/No data in this window/, {}, { timeout: 3_000 })
    ).toBeInTheDocument();
  });

  it("moves to the first field that needs fixing", async () => {
    render(<NewAlertPage />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Create alert" })
    );

    await waitFor(() => expect(screen.getByLabelText("Name")).toHaveFocus());
  });

  it("asks the service what a check would find now, and shows it", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(
      await screen.findByText(
        "Now: meets the condition",
        {},
        { timeout: 3_000 }
      )
    ).toBeInTheDocument();
    expect(screen.getByText("1 of 1 row matches")).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "12" })).toBeInTheDocument();
    expect(alertsClient.previewAlert).toHaveBeenLastCalledWith({
      metric: "id-prs-open",
      range: null,
      condition: ABOVE_TEN,
      context_columns: [],
    });
  });

  it("sends the columns ticked to show in the notification, and previews with them", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(
      storedMetric({
        definition: {
          table: "gold.prs",
          fields: [
            { column: "lead", type: "string", as_name: "lead" },
            { column: "id", type: "int", agg: "count", as_name: "total" },
          ],
        },
      })
    );
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("checkbox", { name: "lead" }));
    expect(screen.getByRole("checkbox", { name: "lead" })).toBeChecked();
    await waitFor(
      () =>
        expect(alertsClient.previewAlert).toHaveBeenLastCalledWith(
          expect.objectContaining({ context_columns: ["lead"] })
        ),
      { timeout: 3_000 }
    );
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith(
        expect.objectContaining({ context_columns: ["lead"] })
      )
    );
  });

  it("ticks at most five columns to show, and keeps the rest from being ticked", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(
      storedMetric({
        definition: {
          table: "gold.prs",
          fields: [
            ...["a", "b", "c", "d", "e", "f"].map((name) => ({
              column: name,
              type: "string",
              as_name: name,
            })),
            { column: "id", type: "int", agg: "count", as_name: "total" },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    for (const name of ["a", "b", "c", "d", "e"]) {
      await userEvent.click(screen.getByRole("checkbox", { name }));
    }

    // WORKAROUND: the kit draws a checkbox as a span, so disabled is aria-disabled.
    const disabled = (name: string) =>
      screen.getByRole("checkbox", { name }).getAttribute("aria-disabled");
    expect(disabled("f")).toBe("true");
    expect(disabled("e")).not.toBe("true");
    await userEvent.click(screen.getByRole("checkbox", { name: "e" }));
    expect(disabled("f")).not.toBe("true");
  });

  it("forgets the columns ticked when another metric is picked", async () => {
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: listed(["prs-open", "prs-stale"]),
      total: 2,
    });
    vi.mocked(customClient.fetchMetric).mockResolvedValue(
      storedMetric({
        definition: {
          table: "gold.prs",
          fields: [
            { column: "lead", type: "string", as_name: "lead" },
            { column: "id", type: "int", agg: "count", as_name: "total" },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("checkbox", { name: "lead" }));
    expect(screen.getByRole("checkbox", { name: "lead" })).toBeChecked();

    await userEvent.clear(screen.getByLabelText("Metric"));
    await userEvent.type(screen.getByLabelText("Metric"), "stale");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-stale" })
    );

    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: "lead" })).not.toBeChecked()
    );
  });

  it("offers no group-by column to show, since the witness always shows it", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(
      storedMetric({
        definition: {
          table: "gold.prs",
          fields: [
            { column: "team", type: "string", as_name: "team" },
            { column: "lead", type: "string", as_name: "lead" },
            { column: "id", type: "int", agg: "count", as_name: "total" },
          ],
          group_by: ["team", "lead"],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );
    await pickColumn(1, "total");

    expect(screen.getByRole("checkbox", { name: "total" })).toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: "team" })).toBeNull();
    expect(screen.queryByRole("checkbox", { name: "lead" })).toBeNull();
  });

  it("shows how many rows of many match, and which", async () => {
    vi.mocked(alertsClient.previewAlert).mockResolvedValue({
      outcome: "breach",
      reason: null,
      witness: {
        matched: 2,
        answered: 12,
        columns: ["repo", "total"],
        rows: [
          ["api", 14],
          ["web", 11],
        ],
      },
    });

    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(
      await screen.findByText("2 of 12 rows match", {}, { timeout: 3_000 })
    ).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "web" })).toBeInTheDocument();
  });

  it("says why a check could not decide before the rule is saved", async () => {
    vi.mocked(alertsClient.previewAlert).mockResolvedValue({
      outcome: "unknown",
      reason: "many_rows",
      witness: null,
    });

    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(
      await screen.findByText(
        "Now: unknown · More than one row",
        {},
        { timeout: 3_000 }
      )
    ).toBeInTheDocument();
  });

  it("does not ask for a preview of a condition still being written", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(<NewAlertPage />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Metric"), "prs");
    await userEvent.click(
      await screen.findByRole("option", { name: "prs-open" })
    );
    await pickColumn(1, "total");
    await vi.advanceTimersByTimeAsync(2_000);

    expect(alertsClient.previewAlert).not.toHaveBeenCalled();
  });

  it("puts a refusal of the preview on the control it names", async () => {
    const said = "`>` does not apply to a text column";
    vi.mocked(alertsClient.previewAlert).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Request validation failed",
        context: {
          field_violations: [
            {
              field: "condition.where.op",
              description: said,
              reason: "INVALID",
            },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();

    expect(
      await screen.findByText(
        "Fix the highlighted fields to check.",
        {},
        { timeout: 3_000 }
      )
    ).toBeInTheDocument();
    expect(condition(1).getByLabelText("Operator")).toHaveAccessibleDescription(
      said
    );
  });

  it("clears a refusal of the condition once the condition changes", async () => {
    const said = "must be at most 1000";
    vi.mocked(alertsClient.createAlert).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Request validation failed",
        context: {
          field_violations: [
            {
              field: "condition.where.value",
              description: said,
              reason: "INVALID",
            },
            {
              field: "interval_secs",
              description: "too short",
              reason: "INVALID",
            },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));
    expect(await screen.findByText(said)).toBeInTheDocument();
    await userEvent.type(condition(1).getByLabelText("Value"), "0");

    expect(screen.queryByText(said)).toBeNull();
    expect(screen.getByText("too short")).toBeInTheDocument();
  });

  it.each([
    [
      "a column the new metric lacks",
      {
        definition: {
          table: "gold.prs",
          fields: [
            { column: "id", type: "int", agg: "count", as_name: "stale" },
            { column: "repo", type: "string", as_name: "repo" },
          ],
        },
      },
      { column: "", op: ">", value: "" },
    ],
    [
      "a column that holds text in the new metric",
      {
        definition: {
          table: "gold.prs",
          fields: [
            { column: "id", type: "string", as_name: "total" },
            { column: "id", type: "int", agg: "count", as_name: "stale" },
          ],
        },
      },
      { column: "", op: ">", value: "" },
    ],
    [
      "one other column, which a new test starts on",
      {
        definition: {
          table: "gold.prs",
          fields: [
            { column: "id", type: "int", agg: "count", as_name: "stale" },
          ],
        },
      },
      { column: "stale", op: ">", value: "" },
    ],
    [
      "a column the new metric holds as a number too",
      {
        definition: {
          table: "gold.prs",
          fields: [{ column: "id", type: "int", agg: "sum", as_name: "total" }],
        },
      },
      { column: "total", op: ">", value: 10 },
    ],
  ])(
    "starts a test over when another metric has %s",
    async (_case, other, kept) => {
      vi.mocked(customClient.fetchMetrics).mockResolvedValue({
        items: listed(["prs-open", "prs-other"]),
        total: 2,
      });
      vi.mocked(customClient.fetchMetric).mockImplementation(async (id) =>
        storedMetric(id === "id-prs-other" ? other : PRS_OPEN)
      );
      vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);

      render(<EditAlertPage id="a1" />, { wrapper });
      await screen.findByLabelText("Value");
      await userEvent.type(screen.getByLabelText("Metric"), "prs");
      await userEvent.click(
        await screen.findByRole("option", { name: "prs-other" })
      );
      await userEvent.click(screen.getByRole("button", { name: "JSON" }));

      await waitFor(() =>
        expect(shownJson().condition, _case).toEqual({
          rows: "one",
          where: kept,
        })
      );
    }
  );

  it("shows no window and no condition before a metric is picked", async () => {
    render(<NewAlertPage />, { wrapper });

    expect(await screen.findByLabelText("Name")).toBeInTheDocument();
    for (const label of ["Check every", "Destination"]) {
      expect(screen.getByLabelText(label), label).toBeEnabled();
    }
    expect(screen.queryByLabelText("Window")).toBeNull();
    expect(screen.queryByRole("group", { name: "Condition" })).toBeNull();
  });

  it.each([
    [
      "one undated column",
      PRS_OPEN,
      { window: false, column: "total", operator: true },
    ],
    [
      "a date to window by",
      DATED_PRS_OPEN,
      { window: true, column: "total", operator: true },
    ],
    [
      "several columns",
      {
        definition: {
          table: "gold.prs",
          fields: [
            { column: "id", type: "int", agg: "count", as_name: "total" },
            { column: "id", type: "int", agg: "max", as_name: "largest" },
          ],
        },
      },
      { window: false, column: "Pick a column", operator: false },
    ],
  ])(
    "draws what a metric with %s offers once it is picked",
    async (_case, metric, drawn) => {
      vi.mocked(customClient.fetchMetric).mockResolvedValue(
        storedMetric(metric)
      );

      render(<NewAlertPage />, { wrapper });
      await userEvent.type(await screen.findByLabelText("Metric"), "prs");
      await userEvent.click(
        await screen.findByRole("option", { name: "prs-open" })
      );

      expect(
        await screen.findByRole("group", { name: "Condition" }),
        _case
      ).toBeInTheDocument();
      expect(
        Boolean(screen.queryByLabelText("Window")),
        `${_case}: window`
      ).toBe(drawn.window);
      await waitFor(() =>
        expect(
          condition(1).getByRole("combobox", { name: "Column" }),
          `${_case}: column`
        ).toHaveTextContent(drawn.column)
      );
      for (const label of ["Operator", "Value"]) {
        expect(
          Boolean(condition(1).queryByLabelText(label)),
          `${_case}: ${label}`
        ).toBe(drawn.operator);
      }
    }
  );

  it("says what is missing before it sends anything", async () => {
    render(<NewAlertPage />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Create alert" })
    );

    expect(await screen.findByText("Enter a name.")).toBeInTheDocument();
    expect(screen.getByText("Pick a metric.")).toBeInTheDocument();
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

  it("puts a refusal of the condition on the test it names, and lists it under the JSON", async () => {
    const said = "the first bound must not be above the second";
    vi.mocked(alertsClient.createAlert).mockRejectedValue(
      new CustomApiError(400, {
        detail: "Request validation failed",
        context: {
          field_violations: [
            {
              field: "condition.where.value",
              description: said,
              reason: "INVALID",
            },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    const value = condition(1).getByLabelText("Value");
    await waitFor(() => expect(value).toHaveAccessibleDescription(said));
    expect(value).toHaveFocus();

    await userEvent.click(screen.getByRole("button", { name: "JSON" }));
    expect(
      screen.getByText("condition.where.value").parentElement
    ).toHaveTextContent(`condition.where.value: ${said}`);
  });

  it("shows a refusal about the whole alert above the form, not as a conflict", async () => {
    vi.mocked(alertsClient.createAlert).mockRejectedValue(
      new CustomApiError(409, {
        context: {
          violations: [
            {
              type: "limit",
              subject: "alerts",
              description: "at most 200 alerts",
            },
          ],
        },
      })
    );

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    expect(await screen.findByText("at most 200 alerts")).toBeInTheDocument();
    expect(
      screen.queryByText(/changed by someone else/)
    ).not.toBeInTheDocument();
  });

  it("says when the metrics cannot be listed", async () => {
    vi.mocked(customClient.fetchMetrics).mockRejectedValue(
      new Error("offline")
    );

    render(<NewAlertPage />, { wrapper });
    await userEvent.click(await screen.findByLabelText("Metric"));

    expect(
      await screen.findByText("Couldn't load metrics.")
    ).toBeInTheDocument();
  });

  it("checks at the interval picked", async () => {
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await pick("Check every", "6 hours");
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));

    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith(
        expect.objectContaining({ interval_secs: 21_600 })
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

describe("the JSON view", { timeout: 20_000 }, () => {
  it("shows the draft the editor holds, as the body the service takes", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "JSON" }));

    expect(shownJson()).toEqual({
      name: "Too many open PRs",
      metric: "id-prs-open",
      condition: ABOVE_TEN,
      context_columns: [],
      range: null,
      interval_secs: 300,
      destination: "ops",
      enabled: true,
    });
  });

  it("hands what is written in it back to the editor", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "JSON" }));
    typeJson(
      JSON.stringify({
        ...shownJson(),
        condition: {
          rows: "one",
          where: {
            any: [
              { column: "total", op: ">", value: 10 },
              { not: { column: "total", op: "between", value: [1, 5] } },
            ],
          },
        },
      })
    );
    await userEvent.click(screen.getByRole("button", { name: "Editor" }));

    const [outer, inner] = screen.getAllByLabelText("Combine");
    expect(outer).toHaveTextContent("Any of");
    expect(inner).toHaveTextContent("Not");
    expect(condition(2).getByLabelText("Operator")).toHaveTextContent(
      "between"
    );
    expect(condition(2).getByLabelText("From")).toHaveValue("1");
    expect(condition(2).getByLabelText("To")).toHaveValue("5");
  });

  it("keeps an unquoted integer past 2^53 as its digits", async () => {
    vi.mocked(alertsClient.createAlert).mockResolvedValue(ALERT);

    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "JSON" }));
    typeJson(
      JSON.stringify({ ...shownJson(), condition: ABOVE_TEN }).replace(
        '"value":10',
        '"value":12345678901234567890'
      )
    );
    await userEvent.click(screen.getByRole("button", { name: "Editor" }));

    expect(condition(1).getByLabelText("Value")).toHaveValue(
      "12345678901234567890"
    );
    await userEvent.click(screen.getByRole("button", { name: "Create alert" }));
    await waitFor(() =>
      expect(alertsClient.createAlert).toHaveBeenCalledWith(
        expect.objectContaining({
          condition: {
            rows: "one",
            where: { column: "total", op: ">", value: "12345678901234567890" },
          },
        })
      )
    );
  });

  it("keeps the rule as it stood while the text does not parse, and will not save it", async () => {
    render(<NewAlertPage />, { wrapper });
    await fillNew();
    await userEvent.click(screen.getByRole("button", { name: "JSON" }));
    typeJson('{ "name": ');

    expect(screen.getByRole("button", { name: "Create alert" })).toBeDisabled();
    expect(screen.getByText("Fix the JSON to save.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Editor" }));
    expect(condition(1).getByLabelText("Value")).toHaveValue("10");
    expect(screen.getByLabelText("Name")).toHaveValue("Too many open PRs");
  });
});

describe("EditAlertPage", () => {
  beforeEach(() => portalRouter.reset("/portal/custom/alerts/a1/edit"));

  it("still shows the rule's condition when its metric cannot be read", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      range: "P7D",
    });
    vi.mocked(customClient.fetchMetric).mockRejectedValue(
      new CustomApiError(404, { detail: "metric prs-open was not found" })
    );

    render(<EditAlertPage id="a1" />, { wrapper });

    expect(
      await screen.findByText("metric prs-open was not found")
    ).toBeInTheDocument();
    expect(condition(1).getByLabelText("Value")).toHaveValue("10");
    expect(
      condition(1).getByRole("combobox", { name: "Column" })
    ).toHaveTextContent("total");
    expect(screen.getByLabelText("Window")).toHaveTextContent("Last 7 days");
  });

  it("replaces the alert at the revision it was opened at", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(ALERT);
    vi.mocked(alertsClient.replaceAlert).mockResolvedValue({
      ...alertAt(20),
      revision: 4,
    });

    render(<EditAlertPage id="a1" />, { wrapper });
    const value = await screen.findByLabelText("Value");
    expect(value).toHaveValue("10");
    await userEvent.clear(value);
    await userEvent.type(value, "20");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(alertsClient.replaceAlert).toHaveBeenCalledWith("a1", {
        name: "Too many open PRs",
        metric: "prs-open",
        condition: {
          rows: "one",
          where: { column: "total", op: ">", value: 20 },
        },
        context_columns: [],
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

  it("opens a nested condition as it was saved and sends it back unchanged", async () => {
    const nested = {
      rows: { count: { op: ">=", value: 2 } },
      where: {
        all: [
          { column: "total", op: "between", value: [1, 9] },
          { not: { any: [{ column: "total", op: "==", value: 4 }] } },
        ],
      },
    } as Alert["condition"];
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      condition: nested,
    });
    vi.mocked(alertsClient.replaceAlert).mockResolvedValue({
      ...ALERT,
      revision: 4,
    });

    render(<EditAlertPage id="a1" />, { wrapper });
    await userEvent.type(await screen.findByLabelText("Name"), "!");
    expect(screen.getByLabelText("Rows")).toHaveTextContent("Number of rows");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(alertsClient.replaceAlert).toHaveBeenCalledWith(
        "a1",
        expect.objectContaining({ condition: nested })
      )
    );
  });

  it("keeps a column to show ticked while the metric no longer answers it", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      context_columns: ["gone"],
    });

    render(<EditAlertPage id="a1" />, { wrapper });

    expect(await screen.findByRole("checkbox", { name: "gone" })).toBeChecked();
  });

  it("seeds the form from a read made now, not from the copy the alert page cached", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(
      alertAt(15, { revision: 4 })
    );
    vi.mocked(alertsClient.replaceAlert).mockResolvedValue(
      alertAt(15, { revision: 5 })
    );

    render(<EditAlertPage id="a1" />, { wrapper: cachedWrapper(ALERT) });

    expect(await screen.findByLabelText("Value")).toHaveValue("15");
    await userEvent.type(screen.getByLabelText("Name"), "!");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(alertsClient.replaceAlert).toHaveBeenCalledWith(
        "a1",
        expect.objectContaining({
          condition: {
            rows: "one",
            where: { column: "total", op: ">", value: 15 },
          },
          expected_revision: 4,
        })
      )
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

  it("sends back a number wider than a JavaScript number, as it came, with a rename", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue(
      alertAt("12345678901234567890")
    );
    vi.mocked(alertsClient.replaceAlert).mockResolvedValue({
      ...ALERT,
      revision: 4,
    });

    render(<EditAlertPage id="a1" />, { wrapper });
    expect(await screen.findByLabelText("Value")).toHaveValue(
      "12345678901234567890"
    );
    await userEvent.type(screen.getByLabelText("Name"), "!");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(alertsClient.replaceAlert).toHaveBeenCalledWith(
        "a1",
        expect.objectContaining({
          name: "Too many open PRs!",
          condition: {
            rows: "one",
            where: { column: "total", op: ">", value: "12345678901234567890" },
          },
        })
      )
    );
  });

  it("drops a window its metric no longer has a date for", async () => {
    vi.mocked(alertsClient.fetchAlert).mockResolvedValue({
      ...ALERT,
      range: "P7D",
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
        expect.objectContaining({ range: null })
      )
    );
  });

  it("keeps the edits when the alert moved on, and offers its latest version", async () => {
    vi.mocked(alertsClient.fetchAlert)
      .mockResolvedValueOnce(ALERT)
      .mockResolvedValue(alertAt(15, { revision: 4 }));
    vi.mocked(alertsClient.replaceAlert).mockRejectedValue(MOVED_ON);

    render(<EditAlertPage id="a1" />, { wrapper });
    const value = await screen.findByLabelText("Value");
    await userEvent.clear(value);
    await userEvent.type(value, "20");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect(
      await screen.findByText(/changed by someone else/)
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Value")).toHaveValue("20");

    await userEvent.click(
      screen.getByRole("button", {
        name: "Discard my edits and load the latest version",
      })
    );
    await waitFor(() =>
      expect(screen.getByLabelText("Value")).toHaveValue("15")
    );
    expect(
      screen.queryByText(/changed by someone else/)
    ).not.toBeInTheDocument();
  });

  it("keeps the edits and the notice when the latest version cannot be read", async () => {
    vi.mocked(alertsClient.fetchAlert)
      .mockResolvedValueOnce(ALERT)
      .mockRejectedValue(
        new CustomApiError(404, { detail: "alert a1 is not there" })
      );
    vi.mocked(alertsClient.replaceAlert).mockRejectedValue(MOVED_ON);

    render(<EditAlertPage id="a1" />, { wrapper });
    const value = await screen.findByLabelText("Value");
    await userEvent.clear(value);
    await userEvent.type(value, "20");
    await userEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Discard my edits and load the latest version",
      })
    );

    expect(
      await screen.findByText("alert a1 is not there")
    ).toBeInTheDocument();
    expect(screen.getByText(/changed by someone else/)).toBeInTheDocument();
    expect(screen.getByLabelText("Value")).toHaveValue("20");
  });
});
