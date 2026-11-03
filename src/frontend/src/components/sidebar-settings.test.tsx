import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

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
  it("offers no switch into the interface the portal replaced", () => {
    render(<SidebarSettings />);

    // Not hidden behind a state: there is no portal preference left to show,
    // and `portal-store` deletes the key it used to live in.
    expect(screen.queryByText("Portal")).not.toBeInTheDocument();
    expect(screen.getByText("Show planned sections")).toBeInTheDocument();
  });
});
