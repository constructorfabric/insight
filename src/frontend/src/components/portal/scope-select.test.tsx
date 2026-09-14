// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  directOnly: false,
  scopeMemberCount: 8,
}));

vi.mock("@/lib/portal/portal-nav", () => ({
  usePortalNavActions: () => ({ setScope: vi.fn() }),
  usePortalScope: () => ({ root: null, directOnly: mocks.directOnly }),
}));

vi.mock("@/lib/portal/use-org-scope", () => ({
  useOrgScope: () => ({
    label: "Manager",
    scopeMemberCount: mocks.scopeMemberCount,
    pivotPersonId: "manager",
    canDirectOnly: true,
    managerNodes: [
      {
        person_id: "manager",
        name: "Manager",
        depth: 0,
        subtreeMemberCount: 8,
        directMemberCount: 3,
      },
      {
        person_id: "nested",
        name: "Nested manager",
        depth: 1,
        subtreeMemberCount: 5,
        directMemberCount: 2,
      },
    ],
  }),
}));

import { ScopeSelect } from "./scope-select";

beforeEach(() => {
  mocks.directOnly = false;
  mocks.scopeMemberCount = 8;
});

describe("ScopeSelect", () => {
  it("counts each manager inside their full scope", async () => {
    const user = userEvent.setup();
    render(<ScopeSelect />);

    expect(
      screen.getByRole("button", { name: "Scope: Manager, 8 people" }),
    ).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Scope: Manager, 8 people" }),
    );

    expect(
      screen.getByRole("button", { name: "Manager, 8 people" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Nested manager, 5 people" }),
    ).toBeInTheDocument();
  });

  it("shows inclusive direct-scope counts in direct reports mode", async () => {
    mocks.directOnly = true;
    mocks.scopeMemberCount = 3;
    const user = userEvent.setup();
    render(<ScopeSelect />);

    await user.click(
      screen.getByRole("button", { name: "Scope: Manager, 3 people" }),
    );

    expect(
      screen.getByRole("button", { name: "Manager, 3 people" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Nested manager, 2 people" }),
    ).toBeInTheDocument();
  });
});
