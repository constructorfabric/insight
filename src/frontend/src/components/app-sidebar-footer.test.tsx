// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { PortalSearch } from "@/lib/portal/portal-search";

let currentPath = "/";
let currentSearch: PortalSearch = {};
let legacyShell = false;

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    search,
    children,
    ...rest
  }: {
    to: string;
    search?: Record<string, unknown>;
    children?: React.ReactNode;
  } & Record<string, unknown>) => (
    <a
      data-testid="link"
      data-to={to}
      // `undefined` is a value here — it CLEARS a key — so JSON would hide it.
      data-search={Object.entries(search ?? {})
        .map(([k, v]) => `${k}=${String(v)}`)
        .join("&")}
      {...rest}
    >
      {children}
    </a>
  ),
  useRouterState: ({
    select,
  }: {
    select: (s: { location: { pathname: string } }) => string;
  }) => select({ location: { pathname: currentPath } }),
  useSearch: () => currentSearch,
}));

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

vi.mock("@/lib/portal/portal-store", () => ({
  readLegacyShell: () => legacyShell,
}));

const mocks = vi.hoisted(() => ({ openFeedback: vi.fn() }));

vi.mock("@/components/feedback-context", () => ({
  useFeedbackDialog: () => ({ openFeedback: mocks.openFeedback }),
}));

vi.mock("@/components/ui/avatar", () => ({
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
  currentSearch = {};
  legacyShell = false;
  mocks.openFeedback.mockClear();
});

describe("AppSidebarFooter", () => {
  it("names the portal's Manage surfaces while the portal is on", () => {
    render(<AppSidebarFooter />);

    expect(linkOf("Metric catalog")).toHaveAttribute("data-to", "/portal");
    expect(linkOf("Metric catalog")).toHaveAttribute(
      "data-search",
      "zone=manage&item=metric-catalog&acct=undefined"
    );
    expect(linkOf("What's new")).toHaveAttribute("data-to", "/portal");
    expect(linkOf("What's new")).toHaveAttribute(
      "data-search",
      "zone=manage&item=whats-new&acct=undefined"
    );
  });

  it("names the standalone screens under the legacy-shell hatch", () => {
    legacyShell = true;
    render(<AppSidebarFooter />);

    expect(linkOf("Metric catalog")).toHaveAttribute("data-to", "/metrics");
    expect(linkOf("What's new")).toHaveAttribute("data-to", "/whats-new");
  });

  it("marks the Manage surface the portal is showing", () => {
    currentSearch = { zone: "manage", item: "whats-new" };
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "true");
    expect(entry("Metric catalog")).toHaveAttribute("data-active", "false");
  });

  it("marks the zone's default when the URL names no item", () => {
    currentSearch = { zone: "manage" };
    render(<AppSidebarFooter />);

    expect(entry("Metric catalog")).toHaveAttribute("data-active", "true");
  });

  it("marks nothing from a zone the portal is not showing", () => {
    // A person route drives the zone from the PATH, not from `?zone=`.
    currentPath = "/ic/019e2800-0000-7000-8000-00000000a11c/personal";
    currentSearch = { zone: "manage", item: "whats-new" };
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });

  it("marks the standalone screen it is standing on under the hatch", () => {
    legacyShell = true;
    currentPath = "/metrics";
    render(<AppSidebarFooter />);

    expect(entry("Metric catalog")).toHaveAttribute("data-active", "true");
    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });

  it("reports a navigation from either destination", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    render(<AppSidebarFooter onNavigate={onNavigate} />);

    await user.click(screen.getByText("Metric catalog"));
    expect(onNavigate).toHaveBeenCalledTimes(1);

    await user.click(screen.getByText("What's new"));
    expect(onNavigate).toHaveBeenCalledTimes(2);
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
