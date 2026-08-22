/**
 * The sparkle is gated twice over, and it is a SIBLING of the tile card.
 * The card renders as a `<button>`; nesting the trigger inside it would be
 * invalid markup that browsers silently reparent.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => {
  const mutate = vi.fn();
  return {
    available: { featureOn: true, hasKey: true },
    mutate,
    explain: {
      mutate,
      isPending: false,
      isError: false,
      data: undefined as
        | undefined
        | {
            text: string;
            model: string;
            tenant_context_entries: number;
            person_context_entries: number;
          },
    },
  };
});

vi.mock("@/queries/ai", () => ({
  useAiAvailable: () => mocks.available,
  useExplainMetric: () => mocks.explain,
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

  it("asks for the explanation the moment the sparkle is pressed", async () => {
    mocks.available = { featureOn: true, hasKey: true };
    const user = userEvent.setup();

    render(<ExplainWithAi snapshot={SNAPSHOT} />);
    await user.click(screen.getByLabelText("Explain Tasks closed with AI"));

    expect(mocks.mutate).toHaveBeenCalledWith(SNAPSHOT);
  });

  it("reads the answer back with the model and the notes behind it", async () => {
    mocks.available = { featureOn: true, hasKey: true };
    mocks.explain.data = {
      text: "Focus time is down seven points.",
      model: "claude-sonnet-5",
      tenant_context_entries: 2,
      person_context_entries: 1,
    };
    const user = userEvent.setup();

    render(<ExplainWithAi snapshot={SNAPSHOT} />);
    await user.click(screen.getByLabelText("Explain Tasks closed with AI"));

    expect(
      screen.getByText("Focus time is down seven points.")
    ).toBeInTheDocument();
    expect(
      screen.getByText(/claude-sonnet-5 · 2 org \+ 1 personal notes/)
    ).toBeInTheDocument();
    mocks.explain.data = undefined;
  });

  it("names the fix when the call comes back refused", async () => {
    mocks.available = { featureOn: true, hasKey: true };
    mocks.explain.isError = true;
    const user = userEvent.setup();

    render(<ExplainWithAi snapshot={SNAPSHOT} />);
    await user.click(screen.getByLabelText("Explain Tasks closed with AI"));

    expect(screen.getByText(/key may have been rejected/)).toBeInTheDocument();
    mocks.explain.isError = false;
  });

  it("shows it is working rather than an empty answer", async () => {
    mocks.available = { featureOn: true, hasKey: true };
    mocks.explain.isPending = true;
    const user = userEvent.setup();

    render(<ExplainWithAi snapshot={SNAPSHOT} />);
    await user.click(screen.getByLabelText("Explain Tasks closed with AI"));

    expect(document.querySelector("[aria-busy]")).not.toBeNull();
    mocks.explain.isPending = false;
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
