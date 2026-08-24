// @vitest-environment jsdom
/**
 * A cell's detail can be a whole feedback submission: what does not fit in the
 * window is unreachable unless the tooltip is bounded and scrolls.
 */
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { TruncatedCell } from "./usage-table";

async function popupFor(trigger: string) {
  await userEvent.hover(screen.getByText(trigger));

  return waitFor(() => {
    const popup = document.querySelector('[data-slot="tooltip-content"]');
    if (!popup) throw new Error("the tooltip has not opened");
    return popup;
  });
}

describe("TruncatedCell", () => {
  it("bounds a long detail to the room the window has, and scrolls the rest", async () => {
    render(
      <TruncatedCell detail={"the whole report. ".repeat(300)}>
        the whole report.
      </TruncatedCell>,
    );

    const popup = await popupFor("the whole report.");

    expect(popup).toHaveClass(
      "max-h-[var(--available-height)]",
      "overflow-y-auto",
    );
  });

  it("keeps the bound when the caller styles the detail", async () => {
    render(
      <TruncatedCell detail="a long message" detailClassName="max-w-sm text-xs">
        a long message
      </TruncatedCell>,
    );

    const popup = await popupFor("a long message");

    expect(popup).toHaveClass("max-h-[var(--available-height)]", "max-w-sm");
  });
});
