// @vitest-environment jsdom
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usageMocks = vi.hoisted(() => ({ recordUsageEvent: vi.fn() }));
const mocks = vi.hoisted(() => ({
  definitions: [] as unknown[],
  scope: {} as Record<string, unknown>,
  dateRange: { from: "2026-05-13", to: "2026-08-12" },
  preview: vi.fn(),
  export: vi.fn(),
  metricVisible: vi.fn(),
  previewPending: false,
  exportPending: false,
}));

vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({
    data: { metrics: mocks.definitions },
    isPending: false,
    isError: false,
  }),
}));
vi.mock("@/lib/portal/use-org-scope", () => ({
  useOrgScope: () => mocks.scope,
}));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({ dateRange: mocks.dateRange }),
}));
vi.mock("@/queries/reports", () => ({
  useReportPreview: () => ({
    isPending: mocks.previewPending,
    mutateAsync: mocks.preview,
  }),
  useReportExport: () => ({
    isPending: mocks.exportPending,
    mutateAsync: mocks.export,
  }),
}));
vi.mock("@/lib/portal/nav-policy", async () => {
  const actual = await vi.importActual<
    typeof import("@/lib/portal/nav-policy")
  >("@/lib/portal/nav-policy");
  return { ...actual, metricVisible: mocks.metricVisible };
});
vi.mock("@/telemetry", async () => {
  const actual =
    await vi.importActual<typeof import("@/telemetry")>("@/telemetry");
  return { ...actual, recordUsageEvent: usageMocks.recordUsageEvent };
});

import { ReportBuilderView } from "@/components/portal/report-builder-view";

const personId = "00000000-0000-0000-0000-000000000001";

const preview = {
  columns: [
    { key: "person", label: "Person", data_type: "text" as const },
    {
      key: "git.commits",
      label: "Commits",
      data_type: "number" as const,
      format: "decimal" as const,
    },
  ],
  rows: [["Jane Doe", 1082.1594444444445]],
  total_rows: 1,
};

function metric(overrides: Record<string, unknown>) {
  return {
    metric_key: "git.commits",
    entity_type: "person",
    label: "Commits",
    short_label: null,
    description: "Authored commits",
    explanation: null,
    unit: null,
    format: "decimal",
    direction: "neutral",
    dimensions: [],
    is_enabled: true,
    origin: "builtin",
    schema_status: "ok",
    schema_error_code: null,
    last_observed_date: "2026-05-01",
    ...overrides,
  };
}

beforeEach(() => {
  mocks.dateRange = { from: "2026-05-13", to: "2026-08-12" };
  mocks.definitions = [
    metric({}),
    metric({ metric_key: "tasks.closed", label: "Issues closed" }),
    metric({ metric_key: "ci.runs", label: "CI runs", entity_type: "tenant" }),
  ];
  mocks.scope = {
    roster: [{ person_id: personId }],
    label: "Whole organisation",
  };
  mocks.preview.mockReset().mockResolvedValue(preview);
  mocks.export.mockReset().mockResolvedValue(undefined);
  mocks.previewPending = false;
  mocks.exportPending = false;
  mocks.metricVisible.mockReset().mockReturnValue(true);
  usageMocks.recordUsageEvent.mockReset();
});

function checkbox(name: string): HTMLElement {
  return screen.getByRole("checkbox", { name });
}

describe("ReportBuilderView", () => {
  it("refuses granularity longer than the period and sends the clamped value", async () => {
    mocks.dateRange = { from: "2026-08-24", to: "2026-08-30" };
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    expect(screen.getByRole("button", { name: "Weekly" })).toHaveAttribute(
      "aria-pressed",
      "true"
    );
    expect(screen.getByRole("button", { name: "Yearly" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Yearly" })).toHaveAttribute(
      "title",
      expect.stringMatching(/at least a year — pick weekly or finer/)
    );

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));

    await waitFor(() =>
      expect(mocks.preview).toHaveBeenCalledWith(
        expect.objectContaining({
          recipe: expect.objectContaining({ granularity: "week" }),
        })
      )
    );
  });

  it("shows applicable metric groups together in the all-metrics tab", () => {
    render(<ReportBuilderView />);

    expect(
      screen.queryByRole("heading", { name: "Report builder" })
    ).toBeNull();
    expect(screen.queryByText(/one row per person per period/)).toBeNull();
    expect(screen.queryByText("Visible roster members")).toBeNull();
    expect(screen.queryByText("Tenant-wide metrics")).toBeNull();
    expect(
      screen.queryByText("Period comes from the bar above")
    ).toBeNull();
    expect(screen.getByText("Scope")).toBeInTheDocument();
    expect(screen.queryByText("Subject")).toBeNull();
    expect(checkbox("Commits")).toBeInTheDocument();
    expect(checkbox("Issues closed")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: "CI runs" })).toBeNull();
    expect(
      screen.getByRole("tab", { name: /All metrics/i })
    ).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /Git/i })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Delivery/i })).toBeInTheDocument();
  });

  it("filters the stacked metric groups with family tabs", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(screen.getByRole("tab", { name: /Git/i }));

    expect(checkbox("Commits")).toBeInTheDocument();
    expect(
      screen.queryByRole("checkbox", { name: "Issues closed" })
    ).toBeNull();

    await user.click(screen.getByRole("tab", { name: /All metrics/i }));

    expect(checkbox("Commits")).toBeInTheDocument();
    expect(checkbox("Issues closed")).toBeInTheDocument();
  });

  it("filters metrics locally by search", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.type(screen.getByRole("searchbox", { name: "Search metrics" }), "issues");

    expect(checkbox("Issues closed")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: "Commits" })).toBeNull();
    expect(mocks.preview).not.toHaveBeenCalled();
  });

  it("keeps selection actions available and clears the current selection", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    expect(
      screen.getByRole("status", { name: "0 selected" })
    ).toBeInTheDocument();
    expect(screen.queryByText("Pick at least one metric")).toBeNull();
    await user.click(checkbox("Commits"));
    expect(
      screen.getByRole("status", { name: "1 selected" })
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Clear" }));

    expect(checkbox("Commits")).not.toBeChecked();
    expect(
      screen.getByRole("status", { name: "0 selected" })
    ).toBeInTheDocument();
  });

  it("omits metric groups hidden by the installation policy", () => {
    mocks.metricVisible.mockImplementation(
      (metricKey) => metricKey !== "tasks.closed"
    );
    render(<ReportBuilderView />);

    expect(checkbox("Commits")).toBeInTheDocument();
    expect(
      screen.queryByRole("checkbox", { name: "Issues closed" })
    ).toBeNull();
    expect(screen.queryByText("Development · Delivery")).toBeNull();
  });

  it("sends visible people IDs in a people recipe", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));

    await waitFor(() =>
      expect(mocks.preview).toHaveBeenCalledWith({
        recipe: {
          subject: { type: "people", ids: [personId] },
          period: mocks.dateRange,
          granularity: "month",
          metric_keys: ["git.commits"],
        },
        signal: expect.any(AbortSignal),
      })
    );
  });

  it("sends tenant subject without people IDs", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(screen.getByRole("button", { name: "Tenant" }));
    await user.click(checkbox("CI runs"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));

    await waitFor(() =>
      expect(mocks.preview).toHaveBeenCalledWith(
        expect.objectContaining({
          recipe: expect.objectContaining({
            subject: { type: "tenant" },
            metric_keys: ["ci.runs"],
          }),
        })
      )
    );
  });

  it("switches the grid to tenant metrics", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(screen.getByRole("button", { name: "Tenant" }));

    expect(checkbox("CI runs")).toBeInTheDocument();
    expect(screen.queryByRole("checkbox", { name: "Commits" })).toBeNull();
    expect(
      screen.queryByRole("checkbox", { name: "Issues closed" })
    ).toBeNull();
  });

  it("does not offer a separate report rows selector", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    expect(screen.queryByText("Rows")).toBeNull();
    expect(screen.queryByRole("button", { name: "Repositories" })).toBeNull();
    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));

    await waitFor(() =>
      expect(mocks.preview).toHaveBeenCalledWith(
        expect.objectContaining({
          recipe: expect.not.objectContaining({ rows: expect.anything() }),
        })
      )
    );
  });

  it("shows server positional values without client rounding", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));

    expect(await screen.findByText("1082.1594444444445")).toBeInTheDocument();
  });

  it("does not render a preview reopen button below the builder", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));
    await screen.findByRole("dialog");
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("button", { name: /1 rows/ })).toBeNull();
  });

  it("exports the current recipe through the reports hook", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));
    await screen.findByRole("dialog");
    await user.click(screen.getByRole("button", { name: "CSV" }));

    await waitFor(() =>
      expect(mocks.export).toHaveBeenCalledWith(
        expect.objectContaining({
          format: "csv",
          recipe: expect.objectContaining({ metric_keys: ["git.commits"] }),
          signal: expect.any(AbortSignal),
        })
      )
    );
    expect(usageMocks.recordUsageEvent).toHaveBeenCalledWith(
      "export",
      "report:csv"
    );
  });

  it("surfaces preview failures without offering a download", async () => {
    mocks.preview.mockRejectedValueOnce(new Error("network"));
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));

    expect(
      await screen.findByText("Could not preview the report: network")
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /rows/ })).toBeNull();
  });

  it("aborts an in-flight preview", async () => {
    let input: { signal?: AbortSignal } | undefined;
    mocks.preview.mockImplementationOnce(
      (next) =>
        new Promise((_, reject) => {
          input = next;
          next.signal.addEventListener("abort", () =>
            reject(new Error("aborted"))
          );
        })
    );
    const user = userEvent.setup();
    const { rerender } = render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));
    await waitFor(() => expect(input).toBeDefined());
    mocks.previewPending = true;
    rerender(<ReportBuilderView />);
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(input?.signal?.aborted).toBe(true);
    expect(
      await screen.findByText("Cancelled — no report was generated.")
    ).toBeInTheDocument();
  });

  it("aborts an in-flight preview when its recipe changes", async () => {
    let input: { signal?: AbortSignal } | undefined;
    mocks.preview.mockImplementationOnce(
      (next) =>
        new Promise((_, reject) => {
          input = next;
          next.signal.addEventListener("abort", () =>
            reject(new Error("aborted"))
          );
        })
    );
    const user = userEvent.setup();
    const { rerender } = render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));
    await user.click(screen.getByRole("button", { name: "Preview report" }));
    await waitFor(() => expect(input).toBeDefined());
    mocks.scope = {
      ...mocks.scope,
      roster: [{ person_id: "00000000-0000-0000-0000-000000000002" }],
    };
    rerender(<ReportBuilderView />);

    expect(input?.signal?.aborted).toBe(true);
  });
  it("requires a narrower people scope when the API roster limit is exceeded", async () => {
    mocks.scope = {
      ...mocks.scope,
      roster: Array.from({ length: 1001 }, () => ({ person_id: personId })),
    };
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    await user.click(checkbox("Commits"));

    expect(
      screen.getByText(
        "This scope has 1,001 people; reports support up to 1,000. Narrow the scope"
      )
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Preview report" })
    ).toBeDisabled();
  });
});
