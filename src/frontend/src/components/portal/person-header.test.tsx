// @vitest-environment jsdom
/**
 * Who the header offers to open.
 *
 * Identity serves a viewer their own subtree, so a person ABOVE them is not
 * theirs to look at. The analytics read path does not enforce that on its own
 * (constructorfabric/insight#1995), which makes the link itself the problem:
 * the UI must not hand out an invitation the backend would answer.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { identityPerson, pid } from "@/test/identity";
import type { IdentityPerson } from "@/types/insight";

const mocks = vi.hoisted(() => ({
  person: undefined as IdentityPerson | undefined,
  managerNodes: [] as Array<{ person_id: string; name: string; depth: number; teamSize: number }>,
  icCalls: [] as string[],
}));

vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: (id: string) => {
    mocks.icCalls.push(id);
    return { data: id === "" ? undefined : mocks.person };
  },
}));
vi.mock("@/lib/portal/use-org-scope", () => ({
  useOrgScope: () => ({ managerNodes: mocks.managerNodes }),
}));

import { PersonHeader } from "./person-header";

const LEAD = pid("lead");
const BOSS = pid("boss");

beforeEach(() => {
  mocks.icCalls = [];
  mocks.managerNodes = [];
  mocks.person = identityPerson("lead", {
    person_id: LEAD,
    display_name: "Lead",
    parent_person_id: BOSS,
    supervisor_name: "Boss",
  });
});

describe("PersonHeader", () => {
  it("does not offer a supervisor the viewer cannot see", () => {
    // The viewer's own subtree holds no node for BOSS, so BOSS is above them.
    render(<PersonHeader person={LEAD} />);
    expect(screen.queryByText("Boss")).not.toBeInTheDocument();
  });

  it("does not even ask identity for a supervisor it will not offer", () => {
    // The siblings dropdown reads the manager's record; skipping the button
    // but still fetching would leave the request the guard exists to avoid.
    render(<PersonHeader person={LEAD} />);
    expect(mocks.icCalls).not.toContain(BOSS);
  });

  it("offers the supervisor once they are inside the viewer's subtree", () => {
    mocks.managerNodes = [
      { person_id: BOSS, name: "Boss", depth: 0, teamSize: 4 },
    ];
    render(<PersonHeader person={LEAD} />);
    expect(screen.getByText("Boss")).toBeInTheDocument();
  });
});

describe("PersonHeader on an organisation with no reporting lines", () => {
  it("offers no team or peer affordance rather than one that dead-ends", () => {
    // Flat shape: no parent to scope to and no manager nodes to resolve, so the
    // hierarchy affordances have nothing to point at. Pinned because the org
    // zones are how a flat organisation navigates — a button here that scoped to
    // an empty org would read as a broken link rather than an absent one.
    mocks.managerNodes = [];
    mocks.person = identityPerson("solo", {
      person_id: pid("solo"),
      display_name: "Solo",
      parent_person_id: null,
    });

    render(<PersonHeader person={pid("solo")} />);

    expect(screen.queryByRole("button", { name: "Team" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Peers" })).toBeNull();
    expect(screen.getByRole("heading", { level: 1 })).toBeInTheDocument();
  });
});
