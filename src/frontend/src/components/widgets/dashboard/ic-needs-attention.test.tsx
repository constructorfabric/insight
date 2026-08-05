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
    medianText: "11 days",
    gapText: "-82%",
    relGap: 0.8,
    ...overrides,
  };
}

describe("IcNeedsAttention", () => {
  it("renders nothing without items", () => {
    const { container } = render(
      <IcNeedsAttention items={[]} onOpenGroup={vi.fn()} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("ranks items by relative gap descending", () => {
    render(
      <IcNeedsAttention
        items={[
          item({ key: "a", label: "Small gap", relGap: 0.1 }),
          item({ key: "b", label: "Large gap", relGap: 2.5 }),
        ]}
        onOpenGroup={vi.fn()}
      />,
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
      />,
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
      />,
    );
    await userEvent.click(screen.getByText("Active AI days"));
    expect(onOpenGroup).toHaveBeenCalledWith("git_output");
  });

  it("collapses beyond the threshold with a show-more toggle", () => {
    const items = Array.from({ length: 9 }, (_, i) =>
      item({ key: `m${i}`, label: `Metric ${i}`, relGap: i }),
    );
    render(<IcNeedsAttention items={items} onOpenGroup={vi.fn()} />);
    expect(screen.getByText("Show 3 more")).toBeInTheDocument();
  });
});
