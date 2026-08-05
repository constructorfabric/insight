import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { IdentityPerson } from "@/types/insight";

let currentPath = "/";
let viewerEmail: string | null = "alice@x.io";
let viewerPersonId: string | null = null;
let viewerData: IdentityPerson | undefined;

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    children,
  }: {
    to: string;
    params?: { person?: string };
    children?: React.ReactNode;
  }) => (
    <a data-testid="link" data-to={to} data-person={params?.person ?? ""}>
      {children}
    </a>
  ),
  useRouterState: ({
    select,
  }: {
    select: (s: { location: { pathname: string } }) => string;
  }) => select({ location: { pathname: currentPath } }),
}));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: viewerEmail, personId: viewerPersonId }),
}));

vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: viewerData }),
}));

vi.mock("@/components/sidebar-settings", () => ({
  SidebarSettings: () => null,
}));

vi.mock("@/components/theme-switcher", () => ({
  ThemeSwitcher: () => null,
}));

vi.mock("@/components/ui/avatar", () => ({
  Avatar: ({ children }: { children?: React.ReactNode }) => (
    <span>{children}</span>
  ),
  AvatarFallback: ({ children }: { children?: React.ReactNode }) => (
    <span data-testid="avatar-fallback">{children}</span>
  ),
}));

vi.mock("@/components/ui/sidebar", () => {
  const passthrough =
    (testId?: string) =>
    ({ children }: { children?: React.ReactNode }) => (
      <div data-testid={testId}>{children}</div>
    );
  return {
    Sidebar: passthrough("sidebar"),
    SidebarContent: passthrough("content"),
    SidebarFooter: passthrough("footer"),
    SidebarGroup: passthrough(),
    SidebarGroupContent: passthrough(),
    SidebarHeader: passthrough(),
    SidebarMenu: passthrough(),
    SidebarMenuItem: passthrough(),
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

import { AppSidebar } from "./app-sidebar";

const PERSON_IDS = {
  alice: "019e2800-0000-7000-8000-00000000a11c",
  bob: "019e2800-0000-7000-8000-00000000b0b0",
  carol: "019e2800-0000-7000-8000-00000000ca01",
  erin: "019e2800-0000-7000-8000-00000000e21e",
} as const;

function person(
  personId: string,
  email: string,
  name: string,
  subordinates: IdentityPerson[] = []
): IdentityPerson {
  return {
    person_id: personId,
    email,
    display_name: name,
    subordinates,
  } as IdentityPerson;
}

// Alice manages Bob (who manages Carol) and Erin.
const tree = person(PERSON_IDS.alice, "alice@x.io", "Alice", [
  person(PERSON_IDS.bob, "bob@x.io", "Bob", [
    person(PERSON_IDS.carol, "carol@x.io", "Carol"),
  ]),
  person(PERSON_IDS.erin, "erin@x.io", "Erin"),
]);

// Scoped to the tree area: the footer repeats the viewer's name/email.
function buttonFor(label: string): HTMLElement | null {
  return within(screen.getByTestId("content"))
    .getByText(label)
    .closest('[data-testid="menu-button"]') as HTMLElement | null;
}

beforeEach(() => {
  currentPath = "/";
  viewerEmail = "alice@x.io";
  viewerPersonId = PERSON_IDS.alice;
  viewerData = tree;
});

describe("AppSidebar", () => {
  it("marks the viewer active on the root path and expands only depth-0", () => {
    render(<AppSidebar />);

    expect(buttonFor("Alice")).toHaveAttribute("data-active", "true");
    expect(buttonFor("Bob")).toHaveAttribute("data-active", "false");
    // Bob's subtree is collapsed: he is neither active nor an ancestor of
    // the active node, so Carol stays hidden.
    expect(screen.queryByText("Carol")).not.toBeInTheDocument();
    expect(screen.getByText("Erin")).toBeInTheDocument();
  });

  it("activates the person from an /ic/ path and opens their subtree", () => {
    currentPath = `/ic/${PERSON_IDS.bob}/personal`;
    render(<AppSidebar />);

    expect(buttonFor("Bob")).toHaveAttribute("data-active", "true");
    // Active node with reports is open -> Carol appears.
    expect(screen.getByText("Carol")).toBeInTheDocument();
    expect(buttonFor("Alice")).toHaveAttribute("data-active", "false");
  });

  it("keeps ancestor chains open when a deep descendant is active", () => {
    currentPath = `/ic/${PERSON_IDS.carol}/personal`;
    render(<AppSidebar />);

    expect(buttonFor("Carol")).toHaveAttribute("data-active", "true");
    expect(buttonFor("Bob")).toHaveAttribute("data-active", "false");
  });

  it("renders no tree while the viewer query has no data and no footer user without an email", () => {
    viewerData = undefined;
    viewerEmail = null;
    viewerPersonId = null;
    render(<AppSidebar />);

    expect(screen.queryByText("Alice")).not.toBeInTheDocument();
    expect(screen.queryByTestId("avatar-fallback")).not.toBeInTheDocument();
  });

  it("shows the viewer's name with the email as secondary line in the footer", () => {
    render(<AppSidebar />);

    const footer = screen.getByTestId("footer");
    expect(footer).toHaveTextContent("Alice");
    expect(footer).toHaveTextContent("alice@x.io");
    expect(screen.getByTestId("avatar-fallback")).toHaveTextContent("A");
  });

  it("falls back to the bare email (no secondary line) when identity is not loaded", () => {
    viewerData = undefined;
    render(<AppSidebar />);

    const footer = screen.getByTestId("footer");
    const emails = screen.getAllByText("alice@x.io");
    expect(footer).toContainElement(emails[0]!);
    expect(emails).toHaveLength(1);
  });

  it("links every person node to their personal dashboard", () => {
    currentPath = `/ic/${PERSON_IDS.carol}/personal`;
    render(<AppSidebar />);

    const links = screen.getAllByTestId("link");
    const personLinks = links.filter(
      (link) => link.dataset.to === "/ic/$person/personal"
    );
    expect(personLinks.map((link) => link.dataset.person)).toEqual([
      PERSON_IDS.alice,
      PERSON_IDS.bob,
      PERSON_IDS.carol,
      PERSON_IDS.erin,
    ]);
  });

  it("marks the what's-new entry active on its route", () => {
    currentPath = "/whats-new";
    render(<AppSidebar />);

    const whatsNew = screen
      .getByText("What's new")
      .closest('[data-testid="menu-button"]') as HTMLElement;
    expect(whatsNew).toHaveAttribute("data-active", "true");
    // No /ic match and not "/" -> no person is active.
    expect(buttonFor("Alice")).toHaveAttribute("data-active", "false");
  });

  it("falls back to the email as the node label when display_name is empty", () => {
    viewerData = person(PERSON_IDS.alice, "alice@x.io", "");
    render(<AppSidebar />);

    expect(buttonFor("alice@x.io")).toBeInTheDocument();
  });

  it("labels a person with neither name nor email so the node stays readable", () => {
    // The identity contract admits both being absent; an empty node would be an
    // unclickable sliver.
    viewerData = person(PERSON_IDS.alice, "", "");
    render(<AppSidebar />);

    expect(buttonFor("Unnamed person")).toBeInTheDocument();
  });

  it("shows the metric catalog entry, active on its route", () => {
    currentPath = "/metrics";
    render(<AppSidebar />);

    const entry = screen
      .getByText("Metric catalog")
      .closest('[data-testid="menu-button"]') as HTMLElement;
    expect(entry).toHaveAttribute("data-active", "true");
  });
});
