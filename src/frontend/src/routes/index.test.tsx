// @vitest-environment jsdom
/**
 * The preview flag can flip while "/" is mounted, and the route's `beforeLoad`
 * guard does not run again for that — only the component can send the reader
 * into the portal without a reload.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    redirect: vi.fn(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/auth", () => ({
  useViewer: () => ({ personId: "person-1" }),
}));
vi.mock("@/screens/dashboard", () => ({
  DashboardScreen: () => <div data-testid="dashboard" />,
}));

import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { setPortalEnabled } from "@/lib/portal/portal-store";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./index";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

beforeEach(() => {
  act(() => {
    setPortalEnabled(false);
    portalRouter.reset("/");
  });
});

describe("/", () => {
  it("renders the dashboard while the preview is off", () => {
    render(<Component />);
    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
    expect(portalRouter.navigations).toEqual([]);
  });

  it("enters the portal when the preview is turned on", () => {
    render(<Component />);
    act(() => setPortalEnabled(true));
    expect(screen.queryByTestId("dashboard")).not.toBeInTheDocument();
    expect(portalRouter.navigations).toEqual([
      { to: "/portal", replace: true },
    ]);
  });
});
