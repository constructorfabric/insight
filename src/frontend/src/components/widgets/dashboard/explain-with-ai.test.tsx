/**
 * The sparkle is gated twice over, and it is a SIBLING of the tile card.
 * The card renders as a `<button>`; nesting the trigger inside it would be
 * invalid markup that browsers silently reparent.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  available: { featureOn: true, hasKey: true },
  mutate: vi.fn(),
}));

vi.mock("@/queries/ai", () => ({
  useAiAvailable: () => mocks.available,
  useExplainMetric: () => ({
    mutate: mocks.mutate,
    isPending: false,
    isError: false,
    data: undefined,
  }),
}));

import { ExplainWithAi } from "./explain-with-ai";
import { KpiTile } from "./kpi-tile";
import type { MetricSnapshot } from "@/api/ai-client";
import type { KpiTileData } from "@/lib/insight/kpi-row";

const SNAPSHOT: MetricSnapshot = {
  metric_key: "tasks.closed",
  label: "Tasks closed",
  value: "34",
  period: "month",
  since: "2026-08-01",
  until: "2026-08-22",
  delta: "+6 since last month",
  peer: "Team median 27",
  help: "",
  trend: [],
};

const TILE: KpiTileData = {
  key: "tasks.closed",
  label: "Tasks closed",
  value: "34",
  delta: null,
  medianLabel: "median 27",
  gapText: null,
  gapStatus: "neutral",
  help: null,
  groupId: null,
};

describe("ExplainWithAi", () => {
  it("draws nothing where the deployment does not offer explanations", () => {
    mocks.available = { featureOn: false, hasKey: true };

    render(<ExplainWithAi snapshot={SNAPSHOT} />);

    expect(screen.queryByLabelText(/explain/i)).not.toBeInTheDocument();
  });

  it("draws nothing until the reader has stored a key", () => {
    mocks.available = { featureOn: true, hasKey: false };

    render(<ExplainWithAi snapshot={SNAPSHOT} />);

    expect(screen.queryByLabelText(/explain/i)).not.toBeInTheDocument();
  });

  it("offers the sparkle once both gates are open", () => {
    mocks.available = { featureOn: true, hasKey: true };

    render(<ExplainWithAi snapshot={SNAPSHOT} />);

    expect(
      screen.getByLabelText("Explain Tasks closed with AI")
    ).toBeInTheDocument();
  });

  it("keeps the sparkle outside the tile's own button", () => {
    mocks.available = { featureOn: true, hasKey: true };

    render(
      <KpiTile
        tile={TILE}
        periodNoun="month"
        onOpenGroup={() => {}}
        explain={SNAPSHOT}
      />
    );

    const sparkle = screen.getByLabelText("Explain Tasks closed with AI");
    expect(sparkle.closest("button:not([aria-label^='Explain'])")).toBeNull();
  });
});
