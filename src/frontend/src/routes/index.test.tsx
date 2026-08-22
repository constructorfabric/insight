// @vitest-environment jsdom
/**
 * "/" is a redirect into the portal for everyone except a document carrying
 * the legacy-shell hatch the stand's UI journeys set — that one still gets the
 * dashboard.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    // The real one throws its own control-flow object; this stands in for it so
    // the test can read the destination back off what was thrown.
    redirect: vi.fn((options: unknown) => ({ redirect: options })),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/auth", () => ({
  useViewer: () => ({ personId: "person-1" }),
}));
vi.mock("@/screens/dashboard", () => ({
  DashboardScreen: () => <div data-testid="dashboard" />,
}));

let legacyShell = false;
vi.mock("@/lib/portal/portal-store", () => ({
  readLegacyShell: () => legacyShell,
}));

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { Route } from "./index";

const route = Route as unknown as {
  beforeLoad: () => void;
  component: () => React.ReactNode;
};

function thrownByBeforeLoad(): unknown {
  try {
    route.beforeLoad();
  } catch (error) {
    return error;
  }
  return undefined;
}

beforeEach(() => {
  legacyShell = false;
});

describe("/", () => {
  it("sends every reader into the portal", () => {
    expect(thrownByBeforeLoad()).toEqual({ redirect: { to: "/portal" } });
  });

  it("lets a legacy-shell document through to the dashboard", () => {
    legacyShell = true;

    expect(thrownByBeforeLoad()).toBeUndefined();
    const Component = route.component;
    render(<Component />);
    expect(screen.getByTestId("dashboard")).toBeInTheDocument();
  });
});
