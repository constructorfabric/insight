const viewer = vi.hoisted(() => ({
  personId: "me-1" as string | null,
  canSeeOthers: false,
  reachPending: false,
}));
vi.mock("@/auth", () => ({
  useViewer: () => ({ personId: viewer.personId }),
}));
vi.mock("@/lib/portal/use-viewer-reach", () => ({
  useViewerReach: () => ({
    canSeeOthers: viewer.canSeeOthers,
    isManager: viewer.canSeeOthers,
    isPending: viewer.reachPending,
  }),
}));
const selectZone = vi.hoisted(() => vi.fn());
vi.mock("@/lib/portal/use-zone-nav", () => ({
  useZoneNav: () => ({ selectZone }),
}));
vi.mock("@/components/portal/metric-groups-view", () => ({
  MetricGroupsView: ({
    personId,
    onSelectGroup,
  }: {
    personId: string;
    onSelectGroup: (id: string) => void;
  }) => (
    <section aria-label={`Glance for ${personId}`}>
      <button type="button" onClick={() => onSelectGroup("git_output")}>
        Git output
      </button>
    </section>
  ),
}));
vi.mock("@/components/portal/home-dashboards", () => ({
  HomeDashboards: () => <section aria-label="Recent dashboards" />,
}));
vi.mock("@/components/portal/domain-lens-view", () => ({
  DomainLensView: ({ config }: { config: { title: string } }) => (
    <section aria-label={`Lens ${config.title}`} />
  ),
  Pending: ({ label }: { label: string }) => <p>{label}</p>,
}));

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { OverviewView } from "./overview-view";

const shownSections = () =>
  screen
    .queryAllByRole("region")
    .map((region) => region.getAttribute("aria-label"));

beforeEach(() => {
  vi.clearAllMocks();
  viewer.personId = "me-1";
  viewer.canSeeOthers = false;
  viewer.reachPending = false;
});

describe("<OverviewView>", () => {
  it.each([
    [
      "an IC: their glance and dashboards, no org rollup",
      { canSeeOthers: false, item: null },
      ["Glance for me-1", "Recent dashboards"],
    ],
    [
      "a viewer with a cohort: their glance, dashboards, then the org rollup",
      { canSeeOthers: true, item: null },
      ["Glance for me-1", "Recent dashboards", "Lens Overview"],
    ],
    [
      "an IC who opens an org theme: Home instead",
      { canSeeOthers: false, item: "trend" },
      ["Glance for me-1", "Recent dashboards"],
    ],
    [
      "a viewer with a cohort who opens an org theme: that theme",
      { canSeeOthers: true, item: "trend" },
      ["Lens Overview · Trend"],
    ],
  ])("shows %s", (_case, { canSeeOthers, item }, expected) => {
    viewer.canSeeOthers = canSeeOthers;

    render(<OverviewView item={item} />);

    expect(shownSections(), `case: ${_case}`).toEqual(expected);
  });

  it("leaves out the glance for a viewer with no person record", () => {
    viewer.personId = null;

    render(<OverviewView item={null} />);

    expect(shownSections()).toEqual(["Recent dashboards"]);
  });

  it("opens a glance tile's section on the viewer's own page", async () => {
    render(<OverviewView item={null} />);

    await userEvent.click(screen.getByRole("button", { name: "Git output" }));

    expect(selectZone).toHaveBeenCalledWith(
      expect.objectContaining({ id: "person" }),
      "git_output"
    );
  });

  it("shows Home without the org rollup while the viewer's reach resolves", () => {
    viewer.reachPending = true;

    render(<OverviewView item={null} />);

    expect(shownSections()).toEqual(["Glance for me-1", "Recent dashboards"]);
  });

  it("holds an org theme while the viewer's reach resolves", () => {
    viewer.reachPending = true;

    render(<OverviewView item="trend" />);

    expect(shownSections()).toEqual([]);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("says so for an item the registry does not have", () => {
    viewer.canSeeOthers = true;

    render(<OverviewView item="nope" />);

    expect(screen.getByText(/isn't an Overview view yet/)).toBeInTheDocument();
  });
});
