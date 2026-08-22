// @vitest-environment jsdom
/**
 * The /portal route is reachable by URL, so the legacy-shell hatch has to be
 * honoured there and not only in the shell that usually renders the portal: a
 * pasted link must not paint a surface the document was told to stay off.
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

let legacyShell = false;
vi.mock("@/lib/portal/portal-store", () => ({
  readLegacyShell: () => legacyShell,
}));

import { render, screen } from "@testing-library/react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

beforeEach(() => {
  legacyShell = false;
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

  it("sends a legacy-shell document home instead", () => {
    legacyShell = true;
    render(<Component />);

    expect(screen.queryByTestId("portal-layout")).not.toBeInTheDocument();
    expect(portalRouter.navigations).toEqual([{ to: "/", replace: true }]);
  });
});
