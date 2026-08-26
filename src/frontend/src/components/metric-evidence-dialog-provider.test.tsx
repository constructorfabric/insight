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
    state:
      | {
          kind: "records";
          activeMetricKey: string;
          targets: Array<{ selection: { metric_key: string } }>;
          title?: string;
        }
      | { kind: "people"; view: { title: string; rows: unknown[] } }
      | null;
    onMetricChange: (key: string) => void;
    onClose: () => void;
  }) => (
    <div>
      <span>
        {state == null
          ? "closed"
          : state.kind === "people"
            ? `people:${state.view.title}`
            : state.activeMetricKey}
      </span>
      <span>{state?.kind === "records" ? state.targets.length : 0}</span>
      <span>{state?.kind === "records" ? state.title : undefined}</span>
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
            { title: "Combined" }
          )
        }
      >
        open many
      </button>
      <button type="button" onClick={() => evidence.openEvidenceTargets([])}>
        open empty
      </button>
      <button
        type="button"
        onClick={() =>
          evidence.openEvidenceTargets(
            [
              { selection: git, label: "Commits" },
              { selection: wiki, label: "Wiki" },
            ],
            { activeMetricKey: wiki.metric_key }
          )
        }
      >
        open on wiki
      </button>
      <button
        type="button"
        onClick={() =>
          evidence.openEvidenceTargets([{ selection: git, label: "Commits" }], {
            activeMetricKey: "not.here",
          })
        }
      >
        open on a stranger
      </button>
      <button
        type="button"
        onClick={() =>
          evidence.openEvidencePeople({
            title: "Commits · 0–50 commits per person",
            metricKey: "git.commits",
            valueLabel: "Commits",
            rows: [
              {
                entityId: "person-a",
                personId: "person-a",
                name: "Ada Lovelace",
                value: 12,
                valueText: "12",
                target: { selection: git, label: "Commits · Ada Lovelace" },
              },
            ],
            allRecords: { selection: git, label: "Commits · all records" },
          })
        }
      >
        open people
      </button>
      <button
        type="button"
        onClick={() =>
          evidence.openEvidencePeople({
            title: "Commits · nobody",
            metricKey: "git.commits",
            valueLabel: "Commits",
            rows: [],
            allRecords: null,
          })
        }
      >
        open nobody
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

  it("opens the people behind a figure, and never an empty list", async () => {
    const user = userEvent.setup();
    render(
      <MetricEvidenceDialogProvider>
        <Controls />
      </MetricEvidenceDialogProvider>
    );

    await user.click(screen.getByRole("button", { name: "open nobody" }));
    // A dialog with nothing in it answers nothing.
    expect(screen.getByText("closed")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "open people" }));
    expect(
      screen.getByText("people:Commits · 0–50 commits per person")
    ).toBeInTheDocument();

    // A metric switch belongs to the records body; it must not disturb a list.
    await user.click(screen.getByRole("button", { name: "select wiki" }));
    expect(
      screen.getByText("people:Commits · 0–50 commits per person")
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "close" }));
    expect(screen.getByText("closed")).toBeInTheDocument();
  });

  it("opens on the metric the caller asked for, not the first one", async () => {
    const user = userEvent.setup();
    render(
      <MetricEvidenceDialogProvider>
        <Controls />
      </MetricEvidenceDialogProvider>
    );

    await user.click(screen.getByRole("button", { name: "open on wiki" }));
    expect(screen.getByText("wiki.pages")).toBeInTheDocument();
  });

  it("falls back to the first metric when the requested one is absent", async () => {
    const user = userEvent.setup();
    render(
      <MetricEvidenceDialogProvider>
        <Controls />
      </MetricEvidenceDialogProvider>
    );

    await user.click(
      screen.getByRole("button", { name: "open on a stranger" })
    );
    expect(screen.getByText("git.commits")).toBeInTheDocument();
  });
});
