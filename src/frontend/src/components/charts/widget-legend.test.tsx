import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { WidgetLegend } from "./widget-legend";

describe("<WidgetLegend>", () => {
  it("lists every series by name", () => {
    render(
      <WidgetLegend
        items={[
          { label: "Organic", color: "var(--chart-1)" },
          { label: "Paid", color: "var(--chart-2)" },
        ]}
      />
    );

    expect(
      screen.getAllByRole("listitem").map((item) => item.textContent)
    ).toEqual(["Organic", "Paid"]);
  });

  it.each([
    [undefined, "dot"],
    ["line", "line"],
    ["dashed", "dashed"],
  ] as const)("marks a %s series with a %s", (style, marker) => {
    render(<WidgetLegend items={[{ label: "Target", color: "red", style }]} />);

    expect(screen.getByRole("listitem").firstElementChild).toHaveAttribute(
      "data-marker",
      marker
    );
  });

  it("draws nothing for no series", () => {
    const { container } = render(<WidgetLegend items={[]} />);

    expect(container).toBeEmptyDOMElement();
  });
});
