// @vitest-environment jsdom
/**
 * What the control says when there is nothing to choose.
 *
 * Slices are discovered by enumerating people and grouping them by one of
 * their attributes, and identity serves a viewer only their own subtree — so a
 * viewer with no reports has a roster of one person and no attribute with a
 * second value. Comparisons still happen (the peer view compares within the
 * organization unit, server-side); the choice is what is missing, and a
 * control offering exactly one option claims the reader chose it.
 */
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/lib/portal/portal-nav", () => ({
  usePortalNavActions: () => ({ setSlice: vi.fn() }),
  usePortalSlice: () => "",
}));

import { SliceSelect } from "./slice-select";

describe("SliceSelect", () => {
  it("does not offer a choice when the roster supports none", () => {
    render(<SliceSelect dims={[]} />);
    const trigger = screen.getByLabelText("Cohort");
    expect(trigger).toBeDisabled();
    // And says why, rather than leaving a dead control.
    expect(trigger).toHaveAttribute(
      "title",
      expect.stringContaining("organization unit"),
    );
  });

  it("is a live control as soon as the roster supports one dimension", () => {
    render(<SliceSelect dims={[{ key: "division", label: "Division" }]} />);
    const trigger = screen.getByLabelText("Cohort");
    expect(trigger).not.toBeDisabled();
    expect(trigger).not.toHaveAttribute("title");
  });
});
