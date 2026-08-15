// @vitest-environment jsdom
/**
 * The footer is rendered by BOTH shells — the legacy sidebar and the portal
 * rail — so its two entries have to name the surface the reader is standing
 * in. Sending a portal reader to /metrics or /whats-new drops the portal
 * chrome and lands them in the previous interface
 * (constructorfabric/insight#2569).
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { PortalSearch } from "@/lib/portal/portal-search";

let currentPath = "/";
let currentSearch: PortalSearch = {};
let portalEnabled = true;

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    search,
    children,
  }: {
    to: string;
    search?: Record<string, unknown>;
    children?: React.ReactNode;
  }) => (
    <a
      data-testid="link"
      data-to={to}
      // `undefined` is a value here (it CLEARS a key), so JSON would hide the
      // very thing under test.
      data-search={Object.entries(search ?? {})
        .map(([k, v]) => `${k}=${String(v)}`)
        .join("&")}
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
  usePortalEnabled: () => portalEnabled,
}));

vi.mock("@/components/ui/avatar", () => ({
  Avatar: ({ children }: { children?: React.ReactNode }) => (
    <span>{children}</span>
  ),
  AvatarFallback: ({ children }: { children?: React.ReactNode }) => (
    <span>{children}</span>
  ),
}));

vi.mock("@/components/ui/sidebar", () => {
  const passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  );
  return {
    SidebarMenu: passthrough,
    SidebarMenuItem: passthrough,
    SidebarMenuButton: ({
      children,
      isActive,
      render: renderProp,
    }: {
      children?: React.ReactNode;
      isActive?: boolean;
      render?: React.ReactNode;
    }) => (
      <div data-testid="menu-button" data-active={String(Boolean(isActive))}>
        {renderProp}
        {children}
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
  portalEnabled = true;
});

describe("AppSidebarFooter", () => {
  it("names the legacy screens while the reader is outside the portal shell", () => {
    render(<AppSidebarFooter />);

    expect(linkOf("Metric catalog")).toHaveAttribute("data-to", "/metrics");
    expect(linkOf("What's new")).toHaveAttribute("data-to", "/whats-new");
  });

  it("names the portal's Manage surfaces while the reader is inside the portal", () => {
    currentPath = "/portal";
    currentSearch = { zone: "overview" };
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

  it("keeps the portal targets on a person route, which the portal shell also owns", () => {
    currentPath = "/ic/019e2800-0000-7000-8000-00000000a11c/personal";
    render(<AppSidebarFooter />);

    expect(linkOf("What's new")).toHaveAttribute("data-to", "/portal");
  });

  it("falls back to the legacy screens when the reader turned the portal off", () => {
    currentPath = "/portal";
    portalEnabled = false;
    render(<AppSidebarFooter />);

    expect(linkOf("What's new")).toHaveAttribute("data-to", "/whats-new");
  });

  it("marks the Manage surface the portal is showing, not the pathname", () => {
    currentPath = "/portal";
    currentSearch = { zone: "manage", item: "whats-new" };
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "true");
    expect(entry("Metric catalog")).toHaveAttribute("data-active", "false");
  });

  it("marks the legacy screen it is standing on outside the portal", () => {
    currentPath = "/metrics";
    render(<AppSidebarFooter />);

    expect(entry("Metric catalog")).toHaveAttribute("data-active", "true");
    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });
});
