// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

let currentPath = "/";

vi.mock("@tanstack/react-router", async () => {
  const { matchesFor } = await import("@/test/portal-router");
  return {
    Link: ({
      to,
      params,
      children,
      ...rest
    }: {
      to: string;
      params?: Record<string, string>;
      children?: React.ReactNode;
    } & Record<string, unknown>) => (
      <a
        data-testid="link"
        data-to={to.replace(/\$(\w+)/g, (_, key: string) => params?.[key] ?? "")}
        {...rest}
      >
        {children}
      </a>
    ),
    useMatches: () => matchesFor(currentPath),
  };
});

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "alice@x.io", personId: null }),
}));

vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: undefined }),
}));

vi.mock("@/components/sidebar-settings", () => ({
  SidebarSettings: () => null,
}));

vi.mock("@/components/theme-switcher", () => ({
  ThemeSwitcher: () => null,
}));

const mocks = vi.hoisted(() => ({ openFeedback: vi.fn() }));

vi.mock("@/components/feedback-context", () => ({
  useFeedbackDialog: () => ({ openFeedback: mocks.openFeedback }),
}));

vi.mock("@gears-frontx/ui-kit", () => ({
  Avatar: ({ children }: { children?: React.ReactNode }) => (
    <span>{children}</span>
  ),
  AvatarFallback: ({ children }: { children?: React.ReactNode }) => (
    <span>{children}</span>
  ),
}));

vi.mock("@/components/ui/sidebar", async () => {
  const { cloneElement, isValidElement } = await import("react");
  const passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  );
  return {
    SidebarMenu: passthrough,
    SidebarMenuItem: passthrough,
    // The real button MERGES into `render`: the label lands inside the anchor.
    SidebarMenuButton: ({
      children,
      isActive,
      onClick,
      render: renderProp,
    }: {
      children?: React.ReactNode;
      isActive?: boolean;
      onClick?: () => void;
      render?: React.ReactNode;
    }) => (
      <div
        data-testid="menu-button"
        data-active={String(Boolean(isActive))}
        onClick={onClick}
      >
        {isValidElement(renderProp)
          ? cloneElement(renderProp, {}, children)
          : children}
      </div>
    ),
  };
});

import { AppSidebarFooter } from "./app-sidebar-footer";

function entry(label: string): HTMLElement {
  return screen
    .getByText(label)
    .closest('[data-testid="menu-button"]') as HTMLElement;
}

function linkOf(label: string): HTMLElement {
  return entry(label).querySelector('[data-testid="link"]') as HTMLElement;
}

beforeEach(() => {
  currentPath = "/";
  mocks.openFeedback.mockClear();
});

describe("AppSidebarFooter", () => {
  it("names the portal's Manage surfaces", () => {
    render(<AppSidebarFooter />);

    expect(linkOf("What's new")).toHaveAttribute("data-to", "/manage/news");
  });

  it("marks the Manage surface the portal is showing", () => {
    currentPath = "/manage/news";
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "true");
  });

  it("marks nothing when Manage shows another surface", () => {
    currentPath = "/manage/assistant";
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });

  it("marks nothing from a zone the portal is not showing", () => {
    currentPath = "/people/019e2800-0000-7000-8000-00000000a11c";
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });

  it("reports a navigation", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    render(<AppSidebarFooter onNavigate={onNavigate} />);

    await user.click(screen.getByText("What's new"));
    expect(onNavigate).toHaveBeenCalledTimes(1);
  });

  it("asks the shell for the feedback dialog and dismisses the menu it sits in", async () => {
    const onNavigate = vi.fn();
    render(<AppSidebarFooter onNavigate={onNavigate} />);

    await userEvent.click(entry("Send feedback"));

    expect(mocks.openFeedback).toHaveBeenCalled();
    expect(onNavigate).toHaveBeenCalled();
  });

  it("offers feedback without navigating anywhere", () => {
    render(<AppSidebarFooter />);

    expect(entry("Send feedback").querySelector('[data-testid="link"]')).toBeNull();
  });

  it("leaves feedback out where the shell already offers it", () => {
    render(<AppSidebarFooter showFeedback={false} />);

    expect(screen.queryByText("Send feedback")).toBeNull();
  });
});
