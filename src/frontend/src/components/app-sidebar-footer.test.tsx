// @vitest-environment jsdom
/**
 * The two entries follow the Portal toggle. They named /metrics and /whats-new
 * unconditionally, which dropped a portal reader into the previous interface
 * (constructorfabric/insight#2569); the toggle stays for now, so the old UI has
 * to remain reachable while it is off.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
    ...rest
  }: {
    to: string;
    search?: Record<string, unknown>;
    children?: React.ReactNode;
  } & Record<string, unknown>) => (
    <a
      data-testid="link"
      data-to={to}
      // `undefined` is a value here (it CLEARS a key), so JSON would hide the
      // very thing under test.
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

vi.mock("@/components/ui/sidebar", async () => {
  const { cloneElement, isValidElement } = await import("react");
  const passthrough = ({ children }: { children?: React.ReactNode }) => (
    <div>{children}</div>
  );
  return {
    SidebarMenu: passthrough,
    SidebarMenuItem: passthrough,
    // The real button MERGES into `render` — the label ends up inside the
    // anchor. Rendering them as siblings would put the label outside the
    // clickable element, so a click test would prove nothing.
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
  portalEnabled = true;
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

  it("names the standalone screens while the portal is off", () => {
    // The toggle stays for now, so the old UI has to remain reachable from
    // the menu that is the only thing linking to it.
    portalEnabled = false;
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
    // `resolveZoneItem` falls back to the first built entry, which is what the
    // context pane highlights — the menu must not disagree with it.
    currentSearch = { zone: "manage" };
    render(<AppSidebarFooter />);

    expect(entry("Metric catalog")).toHaveAttribute("data-active", "true");
  });

  it("marks nothing from a zone the portal is not showing", () => {
    // A person route drives the zone from the PATH, so a `?zone=manage` left
    // behind in the URL names a surface that is not on screen.
    currentPath = "/ic/019e2800-0000-7000-8000-00000000a11c/personal";
    currentSearch = { zone: "manage", item: "whats-new" };
    render(<AppSidebarFooter />);

    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });

  it("marks the standalone screen it is standing on while the portal is off", () => {
    portalEnabled = false;
    currentPath = "/metrics";
    render(<AppSidebarFooter />);

    expect(entry("Metric catalog")).toHaveAttribute("data-active", "true");
    expect(entry("What's new")).toHaveAttribute("data-active", "false");
  });

  // The portal mounts this inside a popover, and the destination now renders
  // behind that popover rather than replacing the shell it sits in. Whoever
  // opened the menu closes it; the footer only says that it navigated.
  it("reports a navigation from either destination", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    render(<AppSidebarFooter onNavigate={onNavigate} />);

    await user.click(screen.getByText("Metric catalog"));
    expect(onNavigate).toHaveBeenCalledTimes(1);
    // The rail collapses only for a pointer, so the pick has to say which it was.
    expect(onNavigate).toHaveBeenLastCalledWith(true);

    await user.click(screen.getByText("What's new"));
    expect(onNavigate).toHaveBeenCalledTimes(2);
  });
});
