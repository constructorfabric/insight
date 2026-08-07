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
    delta: { text: "+17%", status: "good", down: false },
    medianLabel: "median 11",
    gapText: null,
    gapStatus: "neutral",
    help: {
      description: "Days with any AI tool activity",
      explanation: "Counted from tool events, one day per person.",
    },
    groupId: "ai_adoption",
    ...overrides,
  };
}

describe("KpiTile", () => {
  it("renders the display-ready value, delta, median, and context", () => {
    render(<KpiTile tile={tile()} />);
    expect(screen.getByText("14")).toBeInTheDocument();
    expect(screen.getByText("+17%")).toBeInTheDocument();
    expect(screen.getByText(/median 11/)).toBeInTheDocument();
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
    expect(screen.getByText(/3\.5× vs median 3,563/)).toBeInTheDocument();
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

  it("explains the metric on hover, in the catalog's own words", async () => {
    // The number alone is not readable: a viewer meeting "14" has no way to
    // learn what it counts. The tile itself is the trigger, so the answer is
    // one pointer-rest away instead of one more icon per tile.
    render(<KpiTile tile={tile()} onOpenGroup={vi.fn()} />);
    await userEvent.hover(screen.getByRole("button"));
    const tip = await screen.findByTestId("metric-help");
    expect(tip).toHaveTextContent("Days with any AI tool activity");
    expect(tip).toHaveTextContent("Counted from tool events");
  });

  it("opens nothing for a metric the catalog says nothing about", async () => {
    render(<KpiTile tile={tile({ help: null })} onOpenGroup={vi.fn()} />);
    await userEvent.hover(screen.getByRole("button"));
    expect(screen.queryByTestId("metric-help")).not.toBeInTheDocument();
  });

  it("says where the value sits when it is exactly at the median", () => {
    // "median 22,774" alone breaks the pattern every other tile follows and
    // reads as a stray label rather than a comparison.
    render(<KpiTile tile={tile({ gapText: null, medianLabel: "median 11" })} />);
    expect(screen.getByText(/at median 11/)).toBeInTheDocument();
  });

  it("does not raise an alarm over a change of one percent", () => {
    // Four coloured badges per row, one of them for a rounding-sized move,
    // teach the reader that the colour means nothing.
    render(
      <KpiTile
        tile={tile({ delta: { text: "-1%", status: "neutral", down: true } })}
      />,
    );
    const badge = screen.getByText("-1%").closest("span")!;
    expect(badge.className).not.toMatch(/text-destructive|text-success/);
  });
});
