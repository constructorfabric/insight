vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
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
    content,
    onSelectGroup,
  }: {
    personId: string;
    content: string;
    onSelectGroup: (id: string) => void;
  }) => (
    <section
      aria-label={`${content === "attention" ? "Attention" : "Glance"} for ${personId}`}
    >
      <button type="button" onClick={() => onSelectGroup("git_output")}>
        Git output
      </button>
    </section>
  ),
}));
vi.mock("@/components/portal/own-coverage", () => ({
  OwnCoverage: ({ personId }: { personId: string }) => (
    <section aria-label={`Coverage for ${personId}`} />
  ),
}));
vi.mock("@/components/portal/home-dashboards", () => ({
  HomeDashboards: () => <section aria-label="Recent dashboards" />,
}));
vi.mock("@/components/portal/domain-lens-view", () => ({
  DomainLensView: ({
    config,
    personId,
  }: {
    config: { title: string };
    personId?: string | null;
  }) => (
    <section
      aria-label={`Lens ${config.title}${personId ? ` for ${personId}` : ""}`}
    />
  ),
  Pending: ({ label }: { label: string }) => <p>{label}</p>,
}));

import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { zoneItems } from "@/lib/portal/nav-model";
import { homeItemShown } from "@/lib/portal/home-subject";
import { portalRouter } from "@/test/portal-router";

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
  act(() => portalRouter.reset());
});

describe("<OverviewView>", () => {
  it.each([
    [
      "an IC: their glance and dashboards, no org rollup",
      { canSeeOthers: false, who: undefined, item: null },
      ["Glance for me-1", "Recent dashboards"],
    ],
    [
      "a viewer with a cohort: their glance, dashboards, then the org rollup",
      { canSeeOthers: true, who: undefined, item: null },
      ["Glance for me-1", "Recent dashboards", "Lens Overview"],
    ],
    [
      "a viewer with a cohort who picked Me: Home without the rollup",
      { canSeeOthers: true, who: "me", item: null },
      ["Glance for me-1", "Recent dashboards"],
    ],
    [
      "a viewer with a cohort who opens a theme: the team's",
      { canSeeOthers: true, who: undefined, item: "trend" },
      ["Lens Overview · Trend"],
    ],
    [
      "an IC who opens the trend: their own",
      { canSeeOthers: false, who: undefined, item: "trend" },
      ["Lens Trend for me-1"],
    ],
    [
      "a manager on Me who opens By direction: their own",
      { canSeeOthers: true, who: "me", item: "by-direction" },
      ["Lens By direction for me-1"],
    ],
    [
      "an IC who opens Attention needed: their own list",
      { canSeeOthers: false, who: undefined, item: "attention" },
      ["Attention for me-1"],
    ],
    [
      "an IC who opens Data coverage: their own sections",
      { canSeeOthers: false, who: undefined, item: "health" },
      ["Coverage for me-1"],
    ],
    [
      "an IC who opens the split of work between people: Home instead",
      { canSeeOthers: false, who: undefined, item: "contribution" },
      ["Glance for me-1", "Recent dashboards"],
    ],
  ] as const)("shows %s", (_case, { canSeeOthers, who, item }, expected) => {
    viewer.canSeeOthers = canSeeOthers;
    act(() => portalRouter.set({ who }));

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

  it("shows the viewer's own Home while their reach resolves", () => {
    viewer.reachPending = true;

    render(<OverviewView item={null} />);

    expect(shownSections()).toEqual(["Glance for me-1", "Recent dashboards"]);
  });

  it("holds a theme while the viewer's reach resolves", () => {
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

  it.each(
    zoneItems("overview")
      .map((paneItem) => paneItem.id)
      .filter((id) => homeItemShown("me", id))
  )("renders the viewer's own %s rather than a placeholder", (item) => {
    render(<OverviewView item={item} />);

    expect(screen.queryByText(/isn't an Overview view yet/)).toBeNull();
    expect(shownSections().length).toBeGreaterThan(0);
  });
});
