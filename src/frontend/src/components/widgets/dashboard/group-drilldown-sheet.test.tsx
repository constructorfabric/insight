import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  GroupDrilldownSheet,
  type MetricDrilldownTarget,
} from "@/components/widgets/dashboard/group-drilldown-sheet";
import type { MetricGroup } from "@/lib/insight/groups";
import type { MetricCollectionResult } from "@/queries/metric-results";

vi.mock("@/components/widgets/metric-views/collection-drilldown", () => ({
  CollectionDrilldown: () => <div>person-drilldown</div>,
}));
vi.mock("@/components/widgets/metric-views/team-collection-drilldown", () => ({
  TeamCollectionDrilldown: () => <div>team-drilldown</div>,
}));
const METRIC_DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: { metrics: [] },
  card: { preview: [] },
  drilldown: [],
};
const EMPTY_RESULT: MetricCollectionResult = {
  byKey: new Map(),
  previousByKey: null,
  isPending: false,
  isFetching: false,
  isError: false,
  refetch: vi.fn(),
};

function renderSheet(
  def: MetricGroup,
  metricTarget?: MetricDrilldownTarget,
) {
  return render(
    <GroupDrilldownSheet
      open
      onOpenChange={vi.fn()}
      def={def}
      metricTarget={metricTarget}
      range={{ from: "2026-07-01", to: "2026-07-07" }}
      period="week"
    />,
  );
}

describe("GroupDrilldownSheet", () => {
  it("renders the title and the person drilldown for a metrics group", () => {
    renderSheet(METRIC_DEF, {
      kind: "person",
      entityId: "me@x.com",
      data: EMPTY_RESULT,
    });
    expect(screen.getByText("AI adoption")).toBeInTheDocument();
    expect(screen.getByText("person-drilldown")).toBeInTheDocument();
  });

  it("renders the team drilldown for a team target", () => {
    renderSheet(METRIC_DEF, {
      kind: "team",
      members: [],
    });
    expect(screen.getByText("team-drilldown")).toBeInTheDocument();
  });

  it("shows an error when a metrics group has no drilldown target", () => {
    renderSheet(METRIC_DEF, undefined);
    expect(screen.getByText("Missing drilldown data")).toBeInTheDocument();
  });
});
