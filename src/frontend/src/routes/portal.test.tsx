// @vitest-environment jsdom
/**
 * The /portal route is reachable by URL as well as through the shell, and both
 * paint the same layout — there is no preference left that could hold a reader
 * off the portal.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    retainSearchParams: () => () => ({}),
    // The real one registers the route in the generated tree; here it just
    // hands the options back so the test can render the component.
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/components/portal/portal-layout", () => ({
  PortalLayout: () => <div data-testid="portal-layout" />,
}));

import { render, screen } from "@testing-library/react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

beforeEach(() => {
  act(() => {
    portalRouter.reset();
  });
});

describe("/portal", () => {
  it("paints the portal, and sends nobody anywhere else", () => {
    render(<Component />);

    expect(screen.getByTestId("portal-layout")).toBeInTheDocument();
    expect(portalRouter.navigations).toEqual([]);
  });
});
