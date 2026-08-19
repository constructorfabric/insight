// @vitest-environment jsdom
/**
 * The report builder's promises to the reader: never offer a metric the file
 * cannot honestly contain, never offer a file built from something other than
 * what is on screen, and never hand back a partial one.
 */
const usageMocks = vi.hoisted(() => ({ recordUsageEvent: vi.fn() }));

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  definitions: [] as unknown[],
  computations: new Map<string, string>(),
  scope: {} as Record<string, unknown>,
  run: vi.fn(),
  csv: vi.fn(),
  xlsx: vi.fn(),
}));

vi.mock("@/queries/metric-definitions", () => ({
  useMetricDefinitionsResponse: () => ({
    data: { metrics: mocks.definitions },
    isPending: false,
    isError: false,
  }),
}));
vi.mock("@/queries/report-catalogue", () => ({
  useMetricComputations: () => ({
    data: mocks.computations,
    isPending: false,
    isError: false,
  }),
}));
vi.mock("@/lib/portal/use-org-scope", () => ({ useOrgScope: () => mocks.scope }));
vi.mock("@/hooks/use-portal-period", () => ({
  usePortalPeriod: () => ({
    period: "quarter",
    dateRange: { from: "2026-05-13", to: "2026-08-12" },
  }),
}));
vi.mock("@/lib/reports/run-report", () => ({ runReport: mocks.run }));
vi.mock("@/telemetry", async () => {
  const actual = await vi.importActual<typeof import("@/telemetry")>("@/telemetry");
  return { ...actual, recordUsageEvent: usageMocks.recordUsageEvent };
});

vi.mock("@/lib/export/matrix", () => ({
  downloadMatrixCsv: mocks.csv,
  downloadMatrixXlsx: mocks.xlsx,
}));

import { ReportBuilderView } from "@/components/portal/report-builder-view";

const definition = (over: Record<string, unknown>) => ({
  metric_key: "git.commits",
  label: "Commits",
  description: "Authored commits",
  is_enabled: true,
  schema_status: "ok",
  last_observed_date: "2026-05-01",
  origin: "builtin",
  ...over,
});

const person = {
  person_id: "P1",
  email: "jane.doe@example.com",
  display_name: "Jane Doe",
  subordinates: [],
};

beforeEach(() => {
  mocks.definitions = [
    definition({}),
    definition({ metric_key: "git.merge_rate", label: "Merge rate" }),
    definition({
      metric_key: "tasks.closed",
      label: "Issues closed",
      last_observed_date: null,
    }),
  ];
  mocks.computations = new Map([
    ["git.commits", "sum"],
    ["git.merge_rate", "ratio"],
  ]);
  mocks.scope = {
    pivot: person,
    roster: [{ person_id: "P1" }],
    label: "Jane Doe",
    isLoading: false,
    isError: false,
  };
  mocks.run.mockReset().mockResolvedValue(new Map());
  mocks.csv.mockReset();
  mocks.xlsx.mockReset();
});

const box = (name: string) => screen.getByRole("checkbox", { name });
/** The checkbox is a span with a role, so disabled shows as an attribute. */
const isDisabled = (name: string) =>
  box(name).getAttribute("aria-disabled") === "true";
/** The dialog opens over the page and takes the background out of the a11y
 *  tree, so the offer behind it is only reachable once it is closed. */
const closeDialog = (user: ReturnType<typeof userEvent.setup>) =>
  user.keyboard("{Escape}");

describe("ReportBuilderView", () => {
  it("lists a metric nothing has reached, disabled, rather than hiding it", () => {
    // A family that simply vanishes leaves the reader unable to tell "not
    // measured here" from "I misremembered the name".
    render(<ReportBuilderView />);
    expect(isDisabled("Issues closed")).toBe(true);
    expect(screen.getByText("Issues closed").closest("label")).toHaveAttribute(
      "title",
      expect.stringMatching(/No data source is connected/),
    );
  });

  it("disables a non-additive metric only where buckets are added up", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);
    expect(isDisabled("Merge rate")).toBe(false);

    await user.click(screen.getByRole("button", { name: "Yearly" }));
    expect(isDisabled("Merge rate")).toBe(true);
    expect(isDisabled("Commits")).toBe(false);
  });

  it("says why it cannot build yet, rather than greying out in silence", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);

    expect(screen.getByRole("button", { name: "Build report" })).toBeDisabled();
    expect(screen.getByText("Pick at least one metric")).toBeInTheDocument();

    await user.click(box("Commits"));

    expect(screen.queryByText("Pick at least one metric")).not.toBeInTheDocument();
  });

  it("says when the scope is what blocks the build, once a metric is picked", async () => {
    const user = userEvent.setup();
    mocks.scope = { ...mocks.scope, roster: [] };
    render(<ReportBuilderView />);

    await user.click(box("Commits"));

    expect(screen.getByRole("button", { name: "Build report" })).toBeDisabled();
    expect(screen.getByText("This scope has no people")).toBeInTheDocument();
  });

  it("reports which format a reader took the report out in", async () => {
    const user = userEvent.setup();
    usageMocks.recordUsageEvent.mockClear();
    render(<ReportBuilderView />);
    await user.click(box("Commits"));
    await user.click(screen.getByRole("button", { name: "Build report" }));
    await closeDialog(user);
    await user.click(await screen.findByRole("button", { name: /rows ·/ }));

    await user.click(await screen.findByRole("button", { name: "CSV" }));
    expect(usageMocks.recordUsageEvent).toHaveBeenCalledWith("export", "report:csv");

    await user.click(await screen.findByRole("button", { name: "Excel" }));
    expect(usageMocks.recordUsageEvent).toHaveBeenCalledWith("export", "report:xlsx");
  });

  it("cannot build until something is picked", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);
    const build = screen.getByRole("button", { name: "Build report" });
    expect(build).toBeDisabled();

    await user.click(box("Commits"));
    expect(build).not.toBeDisabled();
  });

  it("offers the file once the run has finished, stamped with what made it", async () => {
    const user = userEvent.setup();
    render(<ReportBuilderView />);
    await user.click(box("Commits"));
    await user.click(screen.getByRole("button", { name: "Build report" }));

    // The preview opens on its own — the reader asked for it by building.
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Monthly");
    expect(dialog).toHaveTextContent("2026-05-13 to 2026-08-12");
    expect(mocks.run).toHaveBeenCalledTimes(1);

    await closeDialog(user);
    const offer = await screen.findByRole("button", { name: /rows ·/ });
    expect(offer).toHaveTextContent("Monthly");
  });

  it("withdraws the file when the granularity it was built for changes", async () => {
    // It is held in the screen, not saved. A table built monthly must not
    // still be downloadable under a yearly heading.
    const user = userEvent.setup();
    render(<ReportBuilderView />);
    await user.click(box("Commits"));
    await user.click(screen.getByRole("button", { name: "Build report" }));
    await screen.findByRole("dialog");
    await closeDialog(user);
    await screen.findByRole("button", { name: /rows ·/ });

    await user.click(screen.getByRole("button", { name: "Quarterly" }));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /rows ·/ })).toBeNull(),
    );
  });

  it("withdraws the file when the scope becomes a different roster of the same size", async () => {
    // Counting people would call these the same report. They are not, and the
    // file carries the names of whoever was in scope when it was built.
    const user = userEvent.setup();
    const { rerender } = render(<ReportBuilderView />);
    await user.click(box("Commits"));
    await user.click(screen.getByRole("button", { name: "Build report" }));
    await screen.findByRole("dialog");
    await closeDialog(user);
    await screen.findByRole("button", { name: /rows ·/ });

    mocks.scope = {
      ...mocks.scope,
      pivot: { ...person, person_id: "P2", display_name: "Sam Smith" },
      roster: [{ person_id: "P2" }],
    };
    rerender(<ReportBuilderView />);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: /rows ·/ })).toBeNull(),
    );
  });

  it("downloads nothing when the run fails, and says so", async () => {
    mocks.run.mockRejectedValueOnce(new Error("network"));
    const user = userEvent.setup();
    render(<ReportBuilderView />);
    await user.click(box("Commits"));
    await user.click(screen.getByRole("button", { name: "Build report" }));

    expect(await screen.findByText(/Could not build the report/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /rows ·/ })).toBeNull();
    expect(mocks.csv).not.toHaveBeenCalled();
    expect(mocks.xlsx).not.toHaveBeenCalled();
  });
});
