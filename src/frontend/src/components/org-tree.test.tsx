import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { OrgTree } from "@/components/org-tree";
import type { IdentityPerson } from "@/types/insight";

const mocks = vi.hoisted(() => ({ viewer: null as IdentityPerson | null }));

vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "root" }) }));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: mocks.viewer }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children?: React.ReactNode }) => <a>{children}</a>,
  useRouterState: () => "/portal",
}));
vi.mock("@/lib/portal/portal-nav", () => ({
  usePortalNavActions: () => ({ setScope: vi.fn() }),
}));
vi.mock("@/lib/metrics/entity", () => ({ personIdFromPath: () => null }));
// The sidebar primitives need their provider; the tree's own markup is what
// this file is about, so they stand in as plain elements.
vi.mock("@/components/ui/sidebar", () => ({
  SidebarMenu: ({ children }: { children?: React.ReactNode }) => (
    <ul>{children}</ul>
  ),
  SidebarMenuItem: ({ children }: { children?: React.ReactNode }) => (
    <li>{children}</li>
  ),
  SidebarMenuButton: ({ children }: { children?: React.ReactNode }) => (
    <span>{children}</span>
  ),
}));

function person(
  id: string,
  name: string,
  subordinates: IdentityPerson[] = []
): IdentityPerson {
  return {
    person_id: id,
    email: `${id}@example.com`,
    display_name: name,
    subordinates,
  };
}

mocks.viewer = person("root", "Root Person", [
  person("lead", "Lead Person", [
    person("deep", "Deep Person"),
    person("other", "Other Person"),
  ]),
]);

describe("OrgTree", () => {
  it("shows only the root's own level until something opens it", () => {
    render(<OrgTree />);
    expect(screen.getByText("Root Person")).toBeInTheDocument();
    expect(screen.getByText("Lead Person")).toBeInTheDocument();
    // Nothing is active, so the lead's reports stay folded away.
    expect(screen.queryByText("Deep Person")).not.toBeInTheDocument();
  });

  it("reveals a deep match and the managers above it, and nothing else", () => {
    render(<OrgTree query="Deep" />);
    expect(screen.getByText("Root Person")).toBeInTheDocument();
    expect(screen.getByText("Lead Person")).toBeInTheDocument();
    expect(screen.getByText("Deep Person")).toBeInTheDocument();
    expect(screen.queryByText("Other Person")).not.toBeInTheDocument();
  });

  it("says so when the query names no one, rather than showing an empty tree", () => {
    render(<OrgTree query="nobody" />);
    expect(screen.getByText(/No one here matches/)).toBeInTheDocument();
    expect(screen.queryByText("Root Person")).not.toBeInTheDocument();
  });

  it("goes back to the unfiltered tree when the query is cleared", () => {
    const { rerender } = render(<OrgTree query="Deep" />);
    rerender(<OrgTree query="" />);
    expect(screen.queryByText("Deep Person")).not.toBeInTheDocument();
    expect(screen.getByText("Lead Person")).toBeInTheDocument();
  });
});
