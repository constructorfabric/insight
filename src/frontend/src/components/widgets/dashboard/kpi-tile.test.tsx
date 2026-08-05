import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  KpiTile,
  KpiTilePlaceholder,
} from "@/components/widgets/dashboard/kpi-tile";
import type { KpiTileData } from "@/lib/insight/kpi-row";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all", showExplanations: true }),
}));

function tile(overrides: Partial<KpiTileData> = {}): KpiTileData {
  return {
    key: "ai.active_days",
    label: "Active AI days",
    value: "14",
    valueStatus: "good",
    delta: { text: "+17%", status: "good", down: false },
    medianLabel: "median 11",
    gapText: null,
    gapStatus: "neutral",
    context: "Days with any AI tool activity",
    groupId: "ai_adoption",
    ...overrides,
  };
}

describe("KpiTile", () => {
  it("renders the display-ready value, delta, median, and context", () => {
    render(<KpiTile tile={tile()} />);
    expect(screen.getByText("14")).toBeInTheDocument();
    expect(screen.getByText("+17%")).toBeInTheDocument();
    expect(screen.getByText("median 11")).toBeInTheDocument();
    expect(
      screen.getByText("Days with any AI tool activity"),
    ).toBeInTheDocument();
  });

  it("shows the divergence gap next to the median", () => {
    render(
      <KpiTile
        tile={tile({
          gapText: "3.5×",
          gapStatus: "good",
          medianLabel: "median 3,563",
        })}
      />,
    );
    expect(screen.getByText("3.5×")).toBeInTheDocument();
    expect(screen.getByText(/vs median 3,563/)).toBeInTheDocument();
  });

  it("falls back to 'No peer data' without a median label", () => {
    render(<KpiTile tile={tile({ medianLabel: null })} />);
    expect(screen.getByText("No peer data")).toBeInTheDocument();
  });

  it("omits the delta badge when delta is null", () => {
    render(<KpiTile tile={tile({ delta: null })} />);
    expect(screen.queryByText("+17%")).not.toBeInTheDocument();
  });

  it("navigates to its group on click", async () => {
    const onOpenGroup = vi.fn();
    render(<KpiTile tile={tile()} onOpenGroup={onOpenGroup} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Open Active AI days details" }),
    );
    expect(onOpenGroup).toHaveBeenCalledWith("ai_adoption");
  });

  it("is not interactive without a group id", () => {
    render(<KpiTile tile={tile({ groupId: null })} onOpenGroup={vi.fn()} />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

describe("KpiTilePlaceholder", () => {
  it("renders label-less while a metric tile has no data", () => {
    render(<KpiTilePlaceholder />);
    expect(screen.getByText("Coming soon")).toBeInTheDocument();
  });
});

