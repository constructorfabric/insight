// @vitest-environment jsdom
/**
 * The /portal route is reachable by URL, so the preview flag has to be enforced
 * there and not only in the shell that usually renders the portal. A pasted
 * link — or a viewer who turns the preview off while standing on it — must not
 * paint the portal.
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

import { setPortalEnabled } from "@/lib/portal/portal-store";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

beforeEach(() => {
  act(() => {
    setPortalEnabled(false);
    portalRouter.reset();
  });
});

describe("/portal", () => {
  it("redirects home when the preview is off", () => {
    render(<Component />);
    expect(screen.queryByTestId("portal-layout")).not.toBeInTheDocument();
    expect(portalRouter.navigations).toEqual([{ to: "/", replace: true }]);
  });

  it("renders the portal when the preview is on", () => {
    act(() => setPortalEnabled(true));
    render(<Component />);
    expect(screen.getByTestId("portal-layout")).toBeInTheDocument();
    expect(portalRouter.navigations).toEqual([]);
  });
});
