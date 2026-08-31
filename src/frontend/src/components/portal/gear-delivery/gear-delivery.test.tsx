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

import { GearDeliveryView } from "@/components/portal/gear-delivery-view";
import { GearsTable } from "./gears-table";
import { RoadmapGrid } from "./roadmap-grid";
import { GearSchedule } from "./schedule";
import { GearSummary } from "./summary";

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

describe("GearSummary", () => {
  it("rolls the board up by subsystem", () => {
    render(<GearSummary />);

    const row = screen.getByRole("row", { name: /^CORE/ });

    expect(row).toHaveTextContent("30");
    expect(row).toHaveTextContent("6");
  });

  it("totals every subsystem in a row of its own", () => {
    render(<GearSummary />);

    expect(
      screen.getByRole("row", { name: /All subsystems/ }),
    ).toHaveTextContent("30");
  });

  it("shows a dash where no gear carries that ladder", () => {
    render(<GearSummary />);

    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

  it("says so when the board cannot be read", () => {
    queryState = { isPending: false, isError: true };

    render(<GearSummary />);

    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});

describe("GearsTable", () => {
  it("lists a gear with its estimate and milestone", () => {
    render(<GearsTable />);

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

    render(<GearsTable />);
    await userEvent.type(
      screen.getByRole("searchbox"),
      "dev-two",
    );

    expect(screen.getByText("BSS - Other Module")).toBeInTheDocument();
    expect(screen.queryByText("CORE - Example Module")).not.toBeInTheDocument();
  });
});

describe("RoadmapGrid", () => {
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

    render(<RoadmapGrid />);

    const overdueColumn = screen.getByRole("columnheader", {
      name: "Overdue",
    });
    expect(overdueColumn).toBeInTheDocument();
    expect(screen.getByText(/CORE - Example Module/)).toBeInTheDocument();
    expect(screen.getByText(/CORE - Later/)).toBeInTheDocument();
  });

  it("labels every month of the window", () => {
    render(<RoadmapGrid />);

    expect(
      screen.getByRole("columnheader", { name: "2030-08" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: "2031-04" }),
    ).toBeInTheDocument();
  });
});

describe("GearSchedule", () => {
  it("draws one lane per assignee and states the assumed capacity", () => {
    render(<GearSchedule />);

    expect(screen.getByText("dev-one")).toBeInTheDocument();
    expect(screen.getByText(/1 man-day per calendar day/)).toBeInTheDocument();
  });

  it("says when there is nothing left to schedule", () => {
    queryState = {
      data: roadmap({ lanes: [] }),
      isPending: false,
      isError: false,
    };

    render(<GearSchedule />);

    expect(
      screen.getByText("Nothing is left to schedule."),
    ).toBeInTheDocument();
  });
});

describe("GearDeliveryView", () => {
  it("opens on the summary pane and states the assumed capacity", () => {
    render(<GearDeliveryView config={{ title: "Gear delivery", board: "gear-summary" }} />);

    expect(
      screen.getByText("Capacity assumed: 1 man-day per person per day"),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: "Done %" }),
    ).toBeInTheDocument();
  });

  it("renders the pane the zone item names", () => {
    render(<GearDeliveryView config={{ title: "Gear schedule", board: "gear-schedule" }} />);

    expect(screen.getByText("dev-one")).toBeInTheDocument();
  });
});
