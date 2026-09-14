// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/components/portal/zone-content", () => ({
  ZoneContent: () => <div data-testid="zone-content" />,
}));

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Route as PersonalRoute } from "./ic.$person.personal";
import { Route as TeamRoute } from "./ic.$person.team";

const sources = import.meta.glob("./*.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const routeSources = Object.entries(sources).filter(
  ([path]) => !path.includes(".test.") && !path.endsWith("__root.tsx"),
);

describe("every route", () => {
  it.each(routeSources)("%s paints something or sends the reader on", (_path, source) => {
    expect(
      source.includes("component:") || source.includes("redirect("),
    ).toBe(true);
  });
});

function componentOf(route: unknown): () => React.ReactNode {
  const { component } = route as { component?: () => React.ReactNode };
  if (typeof component !== "function")
    throw new Error("the route paints nothing: the shell's outlet has no child");
  return component;
}

describe.each([
  ["/ic/$person/personal", PersonalRoute],
  ["/ic/$person/team", TeamRoute],
])("%s", (_path, route) => {
  it("paints the zone the shell's outlet asks it for", () => {
    const Zone = componentOf(route);

    render(<Zone />);

    expect(screen.getByTestId("zone-content")).toBeInTheDocument();
  });
});
