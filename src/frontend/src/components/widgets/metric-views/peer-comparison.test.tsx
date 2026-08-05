import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PeerComparison } from "@/components/widgets/metric-views/peer-comparison";
import type { PeerStats } from "@/lib/peers";

const STATS: PeerStats = { p25: 5, p50: 10, p75: 15, min: 0, max: 30, n: 12 };

describe("PeerComparison", () => {
  it("renders the min/max bounds formatted with the metric unit", () => {
    const { getByText } = render(
      <PeerComparison
        value={20}
        stats={STATS}
        status="top"
        higherIsBetter
        format="integer"
        unit="days"
      />,
    );
    expect(getByText("0 days")).toBeInTheDocument();
    expect(getByText("30 days")).toBeInTheDocument();
  });

  it("renders across both zone orientations without throwing", () => {
    for (const higherIsBetter of [true, false]) {
      const { container } = render(
        <PeerComparison
          value={2}
          stats={STATS}
          status={higherIsBetter ? "bottom" : "top"}
          higherIsBetter={higherIsBetter}
          format="percent"
          unit={null}
        />,
      );
      // The marker and the three cohort zones render as positioned children.
      expect(container.querySelectorAll("div").length).toBeGreaterThan(3);
    }
  });
});
