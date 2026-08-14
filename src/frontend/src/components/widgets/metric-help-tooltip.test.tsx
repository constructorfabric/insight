// @vitest-environment jsdom
/**
 * A metric's name carries its meaning to everyone, not only to a mouse.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { MetricName } from "./metric-help-tooltip";

describe("MetricName", () => {
  it("gives the catalog's words to assistive technology, not only to a hover", () => {
    // The tooltip trigger cannot be focusable here — these names sit inside
    // cards that are themselves buttons — so the text has to reach a reader
    // some other way.
    render(
      <MetricName
        metric={{
          label: "Commits",
          description: "Authored commits",
          explanation: "Excludes merge commits.",
        }}
      />
    );
    expect(
      screen.getByText(/Authored commits\. Excludes merge commits\./)
    ).toBeInTheDocument();
  });

  it("says nothing extra when the catalog supplies nothing", () => {
    const { container } = render(<MetricName metric={{ label: "Commits" }} />);
    expect(container.textContent).toBe("Commits");
  });
});
