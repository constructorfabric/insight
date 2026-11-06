// @vitest-environment jsdom
/**
 * A metric's name carries its meaning to everyone, not only to a mouse.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

  it("leads the tooltip with the full name when the surface shows a short one", async () => {
    render(
      <MetricName
        metric={{
          label: "Meetings and Calls Attended",
          description: "Meetings the person joined",
        }}
        text="Meetings"
      />
    );

    await userEvent.hover(screen.getByText("Meetings"));

    const tooltip = await screen.findByTestId("metric-help");
    expect(tooltip.firstElementChild).toHaveTextContent(
      "Meetings and Calls Attended"
    );
  });

  it("gives the full name to assistive technology even without catalog copy", () => {
    render(
      <MetricName
        metric={{ label: "Meetings and Calls Attended" }}
        text="Meetings"
      />
    );
    expect(screen.getByText("Meetings and Calls Attended")).toHaveClass(
      "sr-only"
    );
  });

  it("adds no tooltip of its own when the shown name is the full one", async () => {
    render(<MetricName metric={{ label: "Commits" }} text="Commits" />);

    await userEvent.hover(screen.getByText("Commits"));

    expect(screen.queryByTestId("metric-help")).not.toBeInTheDocument();
  });

  it("names a cut name in full on hover, without repeating it to assistive technology", async () => {
    const { container } = render(
      <MetricName metric={{ label: "Meetings and Calls Attended" }} truncate />
    );

    const name = screen.getByText("Meetings and Calls Attended");
    expect(name).toHaveClass("truncate");
    expect(container.querySelector(".sr-only")).toBeNull();

    await userEvent.hover(name);
    const tooltip = await screen.findByTestId("metric-help");
    expect(tooltip.firstElementChild).toHaveTextContent(
      "Meetings and Calls Attended"
    );
  });

  it("becomes a tab stop only when asked, on a surface that is not a control", () => {
    const metric = { label: "Commits", description: "Every commit pushed" };
    const { rerender } = render(<MetricName metric={metric} />);
    expect(screen.getByText("Commits")).not.toHaveAttribute("tabindex");

    rerender(<MetricName metric={metric} focusable />);
    expect(screen.getByText("Commits")).toHaveAttribute("tabindex", "0");
  });
});
