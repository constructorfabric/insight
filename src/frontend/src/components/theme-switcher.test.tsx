import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { ThemeProvider } from "@/components/theme-provider";

vi.mock("@/components/ui/sidebar", () => ({
  SidebarMenu: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
  SidebarMenuItem: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
  SidebarMenuButton: ({ children }: { children?: React.ReactNode }) => (
    <div data-testid="menu-button">{children}</div>
  ),
}));

vi.mock("@/components/ui/dropdown-menu", () => ({
  DropdownMenu: ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  ),
  DropdownMenuTrigger: ({ render: r }: { render?: React.ReactElement }) => r,
  DropdownMenuContent: ({ children }: { children?: React.ReactNode }) => (
    <div role="menu">{children}</div>
  ),
  DropdownMenuItem: ({
    children,
    onClick,
  }: {
    children?: React.ReactNode;
    onClick?: () => void;
  }) => (
    <div role="menuitem" tabIndex={0} onClick={onClick}>
      {children}
    </div>
  ),
}));

import { ThemeSwitcher } from "./theme-switcher";

function stubMatchMedia() {
  window.matchMedia = ((query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as unknown as MediaQueryList) as typeof window.matchMedia;
}

describe("ThemeSwitcher", () => {
  beforeEach(() => {
    stubMatchMedia();
    localStorage.clear();
    document.documentElement.classList.remove("light", "dark");
  });

  it("shows the current theme and lists all three options", () => {
    render(
      <ThemeProvider defaultTheme="system">
        <ThemeSwitcher />
      </ThemeProvider>,
    );

    expect(screen.getByTestId("menu-button")).toHaveTextContent("System");
    const items = screen.getAllByRole("menuitem");
    expect(items.map((i) => i.textContent)).toEqual([
      "Light",
      "Dark",
      "System",
    ]);
  });

  it("switches the theme when an option is picked", async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider defaultTheme="light">
        <ThemeSwitcher />
      </ThemeProvider>,
    );

    await user.click(screen.getByRole("menuitem", { name: "Dark" }));

    expect(screen.getByTestId("menu-button")).toHaveTextContent("Dark");
    expect(localStorage.getItem("theme")).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});
