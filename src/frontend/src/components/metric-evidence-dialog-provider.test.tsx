import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  useMetricEvidence,
  useMetricEvidenceOptional,
} from "@/components/metric-evidence-context";
import { MetricEvidenceDialogProvider } from "@/components/metric-evidence-dialog-provider";

const mocks = vi.hoisted(() => ({
  session: {
    tenantId: "tenant-a",
    personId: "person-a",
    impersonatorEmail: null,
    roles: ["viewer"],
  } as Record<string, unknown> | null,
  cancelQueries: vi.fn().mockResolvedValue(undefined),
  removeQueries: vi.fn(),
}));

vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: mocks.session }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({
    cancelQueries: mocks.cancelQueries,
    removeQueries: mocks.removeQueries,
  }),
}));

vi.mock("@/components/metric-evidence-dialog", () => ({
  MetricEvidenceDialog: ({
    state,
    onMetricChange,
    onClose,
  }: {
    state: {
      activeMetricKey: string;
      targets: Array<{ selection: { metric_key: string } }>;
      title?: string;
    } | null;
    onMetricChange: (key: string) => void;
    onClose: () => void;
  }) => (
    <div>
      <span>{state?.activeMetricKey ?? "closed"}</span>
      <span>{state?.targets.length ?? 0}</span>
      <span>{state?.title}</span>
      <button type="button" onClick={() => onMetricChange("wiki.pages")}>
        select wiki
      </button>
      <button type="button" onClick={onClose}>
        close
      </button>
    </div>
  ),
}));

const period = { from: "2026-07-01", to: "2026-07-31" };
const git = {
  metric_key: "git.commits",
  entity: { type: "person" as const, id: "person-a" },
  period,
  filters: [],
  display_dimensions: [],
};
const wiki = { ...git, metric_key: "wiki.pages" };

function Controls() {
  const evidence = useMetricEvidence();
  return (
    <>
      <button
        type="button"
        onClick={() => evidence.openEvidence(git, "Commits")}
      >
        open one
      </button>
      <button
        type="button"
        onClick={() =>
          evidence.openEvidenceTargets(
            [
              { selection: git, label: "Commits" },
              { selection: git, label: "Duplicate" },
              { selection: wiki, label: "Wiki" },
            ],
            "Combined"
          )
        }
      >
        open many
      </button>
      <button type="button" onClick={() => evidence.openEvidenceTargets([])}>
        open empty
      </button>
    </>
  );
}

describe("MetricEvidenceDialogProvider", () => {
  it("requires the provider for the strict hook", () => {
    expect(() => render(<Controls />)).toThrow(
      "useMetricEvidence must be used within MetricEvidenceDialogProvider"
    );
    expect(useMetricEvidenceOptional).toBeTypeOf("function");
  });

  it("opens, deduplicates, selects, closes, and clears session-scoped state", async () => {
    const user = userEvent.setup();
    mocks.session = {
      tenantId: "tenant-a",
      personId: "person-a",
      impersonatorEmail: null,
      roles: ["viewer"],
    };
    mocks.cancelQueries.mockClear();
    mocks.removeQueries.mockClear();
    const view = render(
      <MetricEvidenceDialogProvider>
        <Controls />
      </MetricEvidenceDialogProvider>
    );

    await user.click(screen.getByRole("button", { name: "open empty" }));
    expect(screen.getByText("closed")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "open one" }));
    expect(screen.getByText("git.commits")).toBeInTheDocument();
    expect(screen.getByText("1")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "open many" }));
    expect(screen.getByText("2")).toBeInTheDocument();
    expect(screen.getByText("Combined")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "select wiki" }));
    expect(screen.getByText("wiki.pages")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "close" }));
    expect(screen.getByText("closed")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "open one" }));
    mocks.session = {
      tenantId: "tenant-b",
      personId: "person-a",
      impersonatorEmail: null,
      roles: ["viewer"],
    };
    view.rerender(
      <MetricEvidenceDialogProvider>
        <Controls />
      </MetricEvidenceDialogProvider>
    );
    await waitFor(() => expect(screen.getByText("closed")).toBeInTheDocument());
    expect(mocks.cancelQueries).toHaveBeenCalledWith({
      queryKey: ["metric-drilldown"],
    });
    expect(mocks.removeQueries).toHaveBeenCalledWith({
      queryKey: ["metric-drilldown"],
    });
  });
});
