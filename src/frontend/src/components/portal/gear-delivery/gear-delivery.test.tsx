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
import { GearBarCard } from "./bar-card";
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
    issue_url: "https://git.example.test/example-org/example-repo/issues/1",
    assignee_urls: [
      { login: "dev-one", url: "https://git.example.test/dev-one" },
    ],
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

describe("GearBarCard", () => {
  it("describes the gear behind a bar, with links out", () => {
    render(
      <GearBarCard gear={gear()} start="2030-08-01" end="2030-08-06" />,
    );

    expect(screen.getByText("CORE - Example Module")).toBeInTheDocument();
    expect(screen.getByText(/2030-08-01/)).toBeInTheDocument();
    expect(screen.getByText("6 / 30")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open issue #1/ })).toHaveAttribute(
      "href",
      "https://git.example.test/example-org/example-repo/issues/1",
    );
    expect(screen.getByRole("link", { name: "dev-one" })).toHaveAttribute(
      "href",
      "https://git.example.test/dev-one",
    );
  });

  it("falls back to plain logins when no source knows the account", () => {
    render(
      <GearBarCard
        gear={gear({ assignee_urls: [{ login: "dev-one", url: null }] })}
        start="2030-08-01"
        end="2030-08-06"
      />,
    );

    expect(screen.queryByRole("link", { name: "dev-one" })).toBeNull();
    expect(screen.getByText("dev-one")).toBeInTheDocument();
  });
});

describe("GearSchedule bars", () => {
  it("makes every bar a hoverable control named after its gear", () => {
    render(<GearSchedule />);

    expect(
      screen.getByRole("button", { name: "CORE - Example Module" }),
    ).toBeInTheDocument();
  });
});

describe("GearsTable subsystem filter", () => {
  beforeEach(() => {
    queryState = {
      data: roadmap({
        gears: [
          gear({ number: 1, subsystem: "CORE", title: "CORE - One" }),
          gear({ number: 2, subsystem: "BSS", title: "BSS - Two" }),
          gear({ number: 3, subsystem: "BSS", title: "BSS - Three" }),
        ],
      }),
      isPending: false,
      isError: false,
    };
  });

  it("offers one toggle per subsystem on the board, with its count", () => {
    render(<GearsTable />);

    expect(
      screen.getByRole("button", { name: "BSS — 2 gears" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "CORE — 1 gears" })).toBeInTheDocument();
  });

  it("narrows the table to the chosen subsystems", async () => {
    const user = userEvent.setup();
    render(<GearsTable />);

    await user.click(screen.getByRole("button", { name: "BSS — 2 gears" }));

    expect(screen.getByText("BSS - Two")).toBeInTheDocument();
    expect(screen.getByText("BSS - Three")).toBeInTheDocument();
    expect(screen.queryByText("CORE - One")).toBeNull();
  });

  it("shows every gear again once the last subsystem is cleared", async () => {
    const user = userEvent.setup();
    render(<GearsTable />);

    const bss = screen.getByRole("button", { name: "BSS — 2 gears" });
    await user.click(bss);
    await user.click(bss);

    expect(screen.getByText("CORE - One")).toBeInTheDocument();
  });
});
