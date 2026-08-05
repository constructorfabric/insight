import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

vi.mock("@/components/ui/sidebar", () => ({
  SidebarMenu: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
  SidebarMenuItem: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
  SidebarMenuButton: ({
    children,
    onClick,
    "aria-pressed": ariaPressed,
  }: {
    children?: React.ReactNode;
    onClick?: () => void;
    "aria-pressed"?: boolean;
  }) => (
    <div
      data-testid="menu-button"
      role="button"
      tabIndex={0}
      aria-pressed={ariaPressed}
      onClick={onClick}
    >
      {children}
    </div>
  ),
}));

import { SidebarSettings } from "./sidebar-settings";

beforeEach(() => {
  localStorage.clear();
});

describe("SidebarSettings", () => {
  it("shows the focus mode and explanations controls", () => {
    render(<SidebarSettings />);

    expect(screen.getByText("Focus")).toBeInTheDocument();
    expect(screen.getByText("Explanations")).toBeInTheDocument();
    for (const label of ["All", "Critical", "Rewards", "Calm"]) {
      expect(screen.getByRole("button", { name: label })).toBeInTheDocument();
    }
  });

  it("changes the focus mode through the toggle group", async () => {
    const user = userEvent.setup();
    render(<SidebarSettings />);

    await user.click(screen.getByRole("button", { name: "Critical" }));
    expect(localStorage.getItem("insight.focus-mode")).toBe("critical");

    // Clicking the already-selected mode deselects in the widget but the
    // handler ignores the empty value - the mode is retained.
    await user.click(screen.getByRole("button", { name: "Critical" }));
    expect(localStorage.getItem("insight.focus-mode")).toBe("critical");

    await user.click(screen.getByRole("button", { name: "Rewards" }));
    expect(localStorage.getItem("insight.focus-mode")).toBe("rewards");
  });

  it("toggles explanations from its row", async () => {
    const user = userEvent.setup();
    render(<SidebarSettings />);

    const row = screen
      .getByText("Explanations")
      .closest('[data-testid="menu-button"]') as HTMLElement;
    const before = row.getAttribute("aria-pressed");

    await user.click(row);

    const after = before === "true" ? "false" : "true";
    expect(row).toHaveAttribute("aria-pressed", after);
    expect(localStorage.getItem("insight.explanations")).toBe(
      after === "true" ? "true" : "false",
    );
  });
});
