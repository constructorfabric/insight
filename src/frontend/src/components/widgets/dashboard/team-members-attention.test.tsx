/**
 * Component-render coverage for `<TeamMembersAttention>`.
 *
 * Counts come from `metricBelowByMember` (per member's below-peer standings
 * across all groups, each vs their own department cohort); the "worst" headline
 * comes from the member's peer-story entries. Covers:
 *   - a member with a positive count is surfaced with its count + "worst".
 *   - members with a zero count are omitted; an all-zero roster hides the block.
 *   - the list is sorted by count desc and capped at six.
 */

import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    Link: ({ to, children }: { to?: string; params?: unknown; children?: ReactNode }) => (
      <a href={to}>{children}</a>
    ),
  };
});

import type { PeerStoryEntry } from "@/lib/metrics/peer-story";
import type { PeerStatusWithNeutral } from "@/lib/peers";
import { TeamMembersAttention } from "./team-members-attention";
import type { TeamMember } from "@/types/insight";

function makeMember(overrides: Partial<TeamMember> = {}): TeamMember {
  return {
    person_id: "alice@example.com",
    name: "Alice",
    ...overrides,
  };
}

function bottomEntry(label: string, severity: number): PeerStoryEntry {
  return {
    key: label,
    label,
    value: 1,
    unit: null,
    format: "integer",
    higherIsBetter: true,
    neutral: false,
    observed: true,
    stats: { p25: 8, p50: 10, p75: 12, min: 4, max: 20, n: 10 },
    status: "bottom" as PeerStatusWithNeutral,
    gapPct: -0.9,
    gapDelta: -9,
    severity,
  };
}

const NO_ENTRIES = new Map<string, PeerStoryEntry[]>();

describe("<TeamMembersAttention>", () => {
  it("surfaces a member with a positive count and its worst metric", () => {
    render(
      <TeamMembersAttention
        members={[makeMember({ person_id: "alice@example.com", name: "Alice" })]}
        metricBelowByMember={new Map([["alice@example.com", 2]])}
        metricEntriesByPerson={
          new Map([
            [
              "alice@example.com",
              [bottomEntry("Tasks closed", 1), bottomEntry("Focus time", 3)],
            ],
          ])
        }
      />,
    );
    expect(
      screen.getByText("1 members · vs department peers"),
    ).toBeInTheDocument();
    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
    // Worst = the highest-severity bottom entry.
    expect(screen.getByText("worst: Focus time")).toBeInTheDocument();
  });

  it("omits members with a zero count and hides the block when none trail", () => {
    render(
      <TeamMembersAttention
        members={[makeMember()]}
        metricBelowByMember={new Map()}
        metricEntriesByPerson={NO_ENTRIES}
      />,
    );
    expect(
      screen.queryByText("Members needing attention"),
    ).not.toBeInTheDocument();
  });

  it("sorts by count desc and caps at six", () => {
    const members = Array.from({ length: 8 }, (_, i) =>
      makeMember({ person_id: `p${i}@example.com`, name: `P${i}` }),
    );
    // Descending counts p0..p7 = 8..1; only the top six survive the cap.
    const metricBelowByMember = new Map(
      members.map((m, i) => [m.person_id, 8 - i]),
    );
    render(
      <TeamMembersAttention
        members={members}
        metricBelowByMember={metricBelowByMember}
        metricEntriesByPerson={NO_ENTRIES}
      />,
    );
    const names = screen
      .getAllByRole("link")
      .map((a) => a.textContent ?? "")
      .map((t) => (/P\d/.exec(t) ?? [""])[0]);
    expect(names).toEqual(["P0", "P1", "P2", "P3", "P4", "P5"]);
  });
});
