import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { IcNeedsAttention } from "@/components/widgets/dashboard/ic-needs-attention";
import type { AttentionItem } from "@/lib/insight/attention";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all" }),
}));

function item(overrides: Partial<AttentionItem> = {}): AttentionItem {
  return {
    key: "ai.active_days",
    group: "ai_adoption",
    label: "Active AI days",
    valueText: "2 days",
    valueNumber: "2",
    valueUnit: "days",
    medianText: "11 days",
    gapText: "-82%",
    help: null,
    spreadGap: 0.8,
    relGap: 0.8,
    kind: "fell" as const,
    noPrevious: false,
    ...overrides,
  };
}

describe("IcNeedsAttention", () => {
  it("renders nothing without items", () => {
    const { container } = render(
      <IcNeedsAttention items={[]} onOpenGroup={vi.fn()} />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("renders items in the order it is given", () => {
    // Ranking moved to `orderAttentionItems`, next to the rule that produced
    // the items: which of two findings is the stronger is a question about
    // the findings, and the thinning that follows depends on the answer.
    render(
      <IcNeedsAttention
        items={[
          item({ key: "b", label: "Large gap", relGap: 2.5 }),
          item({ key: "a", label: "Small gap", relGap: 0.1 }),
        ]}
        onOpenGroup={vi.fn()}
      />
    );
    const rows = screen.getAllByRole("button");
    expect(rows[0]).toHaveTextContent("Large gap");
    expect(rows[1]).toHaveTextContent("Small gap");
  });

  it("shows the divergence gap next to the median", () => {
    render(
      <IcNeedsAttention
        items={[item({ gapText: "-82%", medianText: "11 days" })]}
        onOpenGroup={vi.fn()}
      />
    );
    const row = screen.getByRole("button");
    expect(row).toHaveTextContent("-82%");
    expect(row).toHaveTextContent("vs median 11 days");
  });

  it("routes clicks to the owning group", async () => {
    const onOpenGroup = vi.fn();
    render(
      <IcNeedsAttention
        items={[item({ group: "git_output" })]}
        onOpenGroup={onOpenGroup}
      />
    );
    await userEvent.click(screen.getByText("Active AI days"));
    expect(onOpenGroup).toHaveBeenCalledWith("git_output");
  });

  it("collapses beyond the threshold with a show-more toggle", () => {
    const items = Array.from({ length: 9 }, (_, i) =>
      item({ key: `m${i}`, label: `Metric ${i}`, relGap: i })
    );
    render(<IcNeedsAttention items={items} onOpenGroup={vi.fn()} />);
    expect(screen.getByText("Show 3 more")).toBeInTheDocument();
  });

  it("explains a row's metric on hover", async () => {
    // The row names a metric and a number; what the metric MEANS lives in the
    // catalog, and the row is the only place a reader meets it.
    render(
      <IcNeedsAttention
        items={[
          item({
            help: {
              description: "Days with any AI tool activity",
              explanation: null,
            },
          }),
        ]}
        onOpenGroup={vi.fn()}
      />
    );
    await userEvent.hover(screen.getByRole("button"));
    expect(await screen.findByTestId("metric-help")).toHaveTextContent(
      "Days with any AI tool activity"
    );
  });
});
