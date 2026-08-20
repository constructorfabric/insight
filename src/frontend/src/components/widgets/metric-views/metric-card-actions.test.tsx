import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  EvidenceDialogContext,
  EvidenceScopeContext,
  type EvidenceDialogTarget,
} from "@/components/metric-evidence-context";
import { MetricCardActions } from "@/components/widgets/metric-views/metric-card-actions";

function selectionFor(metricKey: string) {
  return {
    metric_key: metricKey,
    entity: { type: "person" as const, id: "person-1" },
    period: { from: "2026-07-01", to: "2026-07-31" },
    filters: [],
    display_dimensions: [],
  };
}

function renderActions(scope: EvidenceDialogTarget[]) {
  const openEvidenceTargets = vi.fn();
  render(
    <EvidenceDialogContext.Provider
      value={{ openEvidence: vi.fn(), openEvidenceTargets }}
    >
      <EvidenceScopeContext.Provider value={scope}>
        <MetricCardActions
          evidence={selectionFor("git.prs")}
          label="Pull requests"
        />
      </EvidenceScopeContext.Provider>
    </EvidenceDialogContext.Provider>
  );
  return openEvidenceTargets;
}

async function openMenu() {
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("button", { name: "More actions for Pull requests" })
  );
  await user.click(
    await screen.findByRole("menuitem", { name: "View supporting data" })
  );
}

describe("MetricCardActions", () => {
  it("offers the section's other metrics alongside its own", async () => {
    const openEvidenceTargets = renderActions([
      { selection: selectionFor("git.commits"), label: "Commits" },
      { selection: selectionFor("git.prs"), label: "PRs" },
    ]);

    await openMenu();
    const [targets, options] = openEvidenceTargets.mock.calls[0] ?? [];
    expect(
      (targets as EvidenceDialogTarget[]).map(
        (target) => target.selection.metric_key
      )
    ).toEqual(["git.commits", "git.prs"]);
    expect(options).toEqual({ activeMetricKey: "git.prs" });
  });

  it("opens on its own metric, and with its own label", async () => {
    const openEvidenceTargets = renderActions([
      { selection: selectionFor("git.prs"), label: "Stale label" },
    ]);

    await openMenu();
    const [targets] = openEvidenceTargets.mock.calls[0] ?? [];
    expect((targets as EvidenceDialogTarget[])[0]?.label).toBe("Pull requests");
  });

  it("still opens outside a section, with only itself to offer", async () => {
    const openEvidenceTargets = renderActions([]);

    await openMenu();
    const [targets] = openEvidenceTargets.mock.calls[0] ?? [];
    expect(targets).toHaveLength(1);
  });

  it("renders nothing without a drilldown to open", () => {
    render(
      <EvidenceDialogContext.Provider
        value={{ openEvidence: vi.fn(), openEvidenceTargets: vi.fn() }}
      >
        <MetricCardActions evidence={null} label="Pull requests" />
      </EvidenceDialogContext.Provider>
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
