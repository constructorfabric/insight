import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  TeamCollectionDrilldown,
  type TeamMemberRef,
} from "@/components/widgets/metric-views/team-collection-drilldown";
import type { MetricGroup } from "@/lib/insight/groups";
import type { MemberGridData } from "@/queries/member-grid";

const mocks = vi.hoisted(() => ({
  gridData: vi.fn(),
}));

vi.mock("@/queries/member-grid", () => ({
  useMemberGridData: mocks.gridData,
}));

vi.mock("@/components/widgets/dashboard/members-grid", () => ({
  MembersGrid: ({
    metricKeys,
    members,
  }: {
    metricKeys: string[];
    members: { entityId: string }[];
  }) => (
    <div>
      grid:{metricKeys.join(",")}:{members.length}
    </div>
  ),
}));

const RANGE = { from: "2026-04-20", to: "2026-05-04" };

const DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: {
    metrics: [
      { key: "ai.active_days", views: [{ view: "period" }, { view: "peer" }] },
      {
        key: "ai.accepted_lines",
        views: [{ view: "period" }, { view: "peer" }],
      },
    ],
  },
  card: { preview: [] },
  drilldown: [],
};

const MEMBERS: TeamMemberRef[] = [
  { entityId: "a@x.com", displayName: "Ann" },
  { entityId: "b@x.com", displayName: "Bo" },
];

function gridData(overrides: Partial<MemberGridData> = {}): MemberGridData {
  return {
    byKey: new Map(),
    previousByKey: new Map(),
    isPending: false,
    isFetching: false,
    isError: false,
    refetch: vi.fn(),
    ...overrides,
  };
}

function renderDrilldown(
  data: MemberGridData,
  members: TeamMemberRef[] = MEMBERS
) {
  mocks.gridData.mockReturnValue(data);
  return render(
    <TeamCollectionDrilldown
      def={DEF}
      members={members}
      range={RANGE}
      period="week"
    />
  );
}

describe("TeamCollectionDrilldown", () => {
  it("renders the members grid over the group's full collection", () => {
    renderDrilldown(gridData());
    expect(
      screen.getByText("grid:ai.active_days,ai.accepted_lines:2")
    ).toBeInTheDocument();
  });

  it("shows a spinner while the grid data loads", () => {
    const { container } = renderDrilldown(gridData({ isPending: true }));
    expect(container.querySelector("svg")).toBeInTheDocument();
  });

  it("shows an error with retry when the fetch fails", () => {
    renderDrilldown(gridData({ isError: true }));
    expect(screen.getByText("Unable to load metrics")).toBeInTheDocument();
  });

  it("shows the no-members message when the roster is empty", () => {
    renderDrilldown(gridData(), []);
    expect(screen.getByText(/No team members/i)).toBeInTheDocument();
  });
});
