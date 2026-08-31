import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { Gear, GearRoadmap } from "@/api/gear-roadmap-client";

let queryState: {
  data?: GearRoadmap;
  isPending: boolean;
  isError: boolean;
};

vi.mock("@/queries/gear-roadmap", () => ({
  useGearRoadmap: () => queryState,
}));

vi.mock("@/components/ui/sidebar", () => ({
  SidebarTrigger: () => null,
}));

import { GearGanttScreen } from "./gear-gantt";
import { GearItemsScreen } from "./gear-items";
import { GearOverviewScreen } from "./gear-overview";
import { GearRoadmapGridScreen } from "./gear-roadmap-grid";

function gear(over: Partial<Gear> = {}): Gear {
  return {
    number: 1,
    title: "CORE - Example Module",
    subsystem: "CORE",
    status_percent: 80,
    design_percent: 100,
    sdk_percent: null,
    commitment: "committed",
    priority: "1 (1.1.1)",
    effort_man_days: 30,
    remaining_man_days: 6,
    milestone: "2030-09",
    placement: "slot",
    slot: 1,
    assignees: ["dev-one"],
    closed: false,
    ...over,
  };
}

function roadmap(over: Partial<GearRoadmap> = {}): GearRoadmap {
  return {
    capacity_man_days_per_person: 1,
    window_start: "2030-08",
    window_months: 9,
    gears: [gear()],
    lanes: [
      {
        assignee: "dev-one",
        spans: [{ gear_number: 1, start: "2030-08-01", end: "2030-08-06" }],
      },
    ],
    ...over,
  };
}

beforeEach(() => {
  queryState = { data: roadmap(), isPending: false, isError: false };
});

describe("GearOverviewScreen", () => {
  it("rolls the board up by subsystem", () => {
    render(<GearOverviewScreen />);

    expect(screen.getByText("CORE")).toBeInTheDocument();
    expect(screen.getByText("30")).toBeInTheDocument();
    expect(screen.getByText("6")).toBeInTheDocument();
  });

  it("shows a dash where no gear carries that ladder", () => {
    render(<GearOverviewScreen />);

    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("says so when the board cannot be read", () => {
    queryState = { isPending: false, isError: true };

    render(<GearOverviewScreen />);

    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});

describe("GearItemsScreen", () => {
  it("lists a gear with its estimate and milestone", () => {
    render(<GearItemsScreen />);

    expect(screen.getByText("CORE - Example Module")).toBeInTheDocument();
    expect(screen.getByText("2030-09")).toBeInTheDocument();
  });

  it("filters by assignee", async () => {
    queryState = {
      data: roadmap({
        gears: [
          gear({ number: 1, assignees: ["dev-one"] }),
          gear({
            number: 2,
            title: "BSS - Other Module",
            assignees: ["dev-two"],
          }),
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<GearItemsScreen />);
    await userEvent.type(
      screen.getByRole("searchbox"),
      "dev-two",
    );

    expect(screen.getByText("BSS - Other Module")).toBeInTheDocument();
    expect(screen.queryByText("CORE - Example Module")).not.toBeInTheDocument();
  });
});

describe("GearRoadmapGridScreen", () => {
  it("gives overdue work its own column, apart from later work", () => {
    queryState = {
      data: roadmap({
        gears: [
          gear({ number: 1, placement: "overdue", slot: null }),
          gear({ number: 2, title: "CORE - Later", placement: "backlog" }),
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<GearRoadmapGridScreen />);

    const overdueColumn = screen.getByRole("columnheader", {
      name: "Overdue",
    });
    expect(overdueColumn).toBeInTheDocument();
    expect(screen.getByText(/CORE - Example Module/)).toBeInTheDocument();
    expect(screen.getByText(/CORE - Later/)).toBeInTheDocument();
  });

  it("labels every month of the window", () => {
    render(<GearRoadmapGridScreen />);

    expect(
      screen.getByRole("columnheader", { name: "2030-08" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: "2031-04" }),
    ).toBeInTheDocument();
  });
});

describe("GearGanttScreen", () => {
  it("draws one lane per assignee and states the assumed capacity", () => {
    render(<GearGanttScreen />);

    expect(screen.getByText("dev-one")).toBeInTheDocument();
    expect(screen.getByText(/1 man-day per calendar day/)).toBeInTheDocument();
  });

  it("says when there is nothing left to schedule", () => {
    queryState = {
      data: roadmap({ lanes: [] }),
      isPending: false,
      isError: false,
    };

    render(<GearGanttScreen />);

    expect(
      screen.getByText("Nothing is left to schedule."),
    ).toBeInTheDocument();
  });
});
