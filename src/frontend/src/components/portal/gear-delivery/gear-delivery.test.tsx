import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

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
    placement: { kind: "slot", slot: 1 },
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
        assignee_url: "https://git.example.test/dev-one",
        spans: [{ gear_number: 1, start: "2030-08-01", end: "2030-08-06" }],
      },
    ],
    ...over,
  };
}

beforeEach(() => {
  portalRouter.reset();
  queryState = { data: roadmap(), isPending: false, isError: false };
});

describe("GearSummary", () => {
  it("rolls the board up by subsystem", () => {
    render(<GearSummary roadmap={queryState.data!} />);

    const row = screen.getByRole("row", { name: /^CORE/ });

    expect(row).toHaveTextContent("30");
    expect(row).toHaveTextContent("6");
  });

  it("totals every subsystem in a row of its own", () => {
    render(<GearSummary roadmap={queryState.data!} />);

    expect(
      screen.getByRole("row", { name: /All subsystems/ }),
    ).toHaveTextContent("30");
  });

  it("shows a dash where no gear carries that ladder", () => {
    render(<GearSummary roadmap={queryState.data!} />);

    expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  });

});

describe("GearsTable", () => {
  it("lists a gear with its estimate and milestone", () => {
    render(<GearsTable roadmap={queryState.data!} />);

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

    render(<GearsTable roadmap={queryState.data!} />);
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
          gear({ number: 1, placement: { kind: "overdue", days: 12 } }),
          gear({ number: 2, title: "CORE - Later", placement: { kind: "backlog" } }),
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<RoadmapGrid roadmap={queryState.data!} />);

    const overdueColumn = screen.getByRole("columnheader", {
      name: "Overdue",
    });
    expect(overdueColumn).toBeInTheDocument();
    expect(screen.getByText(/CORE - Example Module/)).toBeInTheDocument();
    expect(screen.getByText(/CORE - Later/)).toBeInTheDocument();
  });

  it("labels every month of the window", () => {
    render(<RoadmapGrid roadmap={queryState.data!} />);

    expect(
      screen.getByRole("columnheader", { name: "2030-08" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: "2031-04" }),
    ).toBeInTheDocument();
  });

  it("links a gear to its issue", () => {
    render(<RoadmapGrid roadmap={queryState.data!} />);

    expect(
      screen.getByRole("link", { name: "CORE - Example Module" }),
    ).toHaveAttribute(
      "href",
      "https://git.example.test/example-org/example-repo/issues/1",
    );
  });

  it("leaves a gear no source claims as plain text, linking the rest", () => {
    queryState = {
      data: roadmap({
        gears: [
          gear({ number: 1 }),
          gear({ number: 2, title: "CORE - Unclaimed", issue_url: null }),
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<RoadmapGrid roadmap={queryState.data!} />);

    expect(screen.getAllByRole("link")).toHaveLength(1);
    expect(screen.getByRole("link").textContent).toContain(
      "CORE - Example Module",
    );
    expect(screen.getByText("CORE - Unclaimed")).toBeInTheDocument();
  });
});

describe("GearSchedule", () => {
  it("links the lane to the account page of the person in it", () => {
    render(<GearSchedule roadmap={queryState.data!} />);

    expect(screen.getByRole("link", { name: "dev-one" })).toHaveAttribute(
      "href",
      "https://git.example.test/dev-one",
    );
  });

  it("leaves an unlinked lane as plain text", () => {
    queryState = {
      data: roadmap({
        lanes: [
          {
            assignee: "dev-two",
            spans: [
              { gear_number: 1, start: "2030-08-01", end: "2030-08-06" },
            ],
          },
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<GearSchedule roadmap={queryState.data!} />);

    expect(screen.queryByRole("link", { name: "dev-two" })).toBeNull();
    expect(screen.getByText("dev-two")).toBeInTheDocument();
  });

  it("draws one lane per assignee and states the assumed capacity", () => {
    render(<GearSchedule roadmap={queryState.data!} />);

    expect(screen.getByRole("link", { name: "dev-one" })).toBeInTheDocument();
    expect(screen.getByText(/1 man-day per calendar day/)).toBeInTheDocument();
  });

  it("says when there is nothing left to schedule", () => {
    queryState = {
      data: roadmap({ lanes: [] }),
      isPending: false,
      isError: false,
    };

    render(<GearSchedule roadmap={queryState.data!} />);

    expect(
      screen.getByText("Nothing is left to schedule."),
    ).toBeInTheDocument();
  });
});

describe("GearDeliveryView", () => {
  it("says so when the board cannot be read", () => {
    queryState = { isPending: false, isError: true };

    render(
      <GearDeliveryView
        config={{ title: "Gear summary", board: "gear-summary" }}
      />,
    );

    expect(screen.getByRole("alert")).toBeInTheDocument();
  });


  it("opens on the summary pane and states the assumed capacity", () => {
    render(<GearDeliveryView config={{ title: "Gear summary", board: "gear-summary" }} />);

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
    render(<GearSchedule roadmap={queryState.data!} />);

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

  it("opens on every subsystem, one control wide", () => {
    render(<GearsTable roadmap={queryState.data!} />);

    expect(
      screen.getByRole("combobox", { name: "Subsystem" }),
    ).toHaveTextContent("All subsystems");
  });

  it("narrows the table to the chosen subsystem", async () => {
    const user = userEvent.setup();
    render(<GearsTable roadmap={queryState.data!} />);

    await user.click(screen.getByRole("combobox", { name: "Subsystem" }));
    await user.click(await screen.findByRole("option", { name: /^BSS/ }));

    expect(screen.getByText("BSS - Two")).toBeInTheDocument();
    expect(screen.getByText("BSS - Three")).toBeInTheDocument();
    expect(screen.queryByText("CORE - One")).toBeNull();
  });

  it("counts the gears each subsystem holds", async () => {
    const user = userEvent.setup();
    render(<GearsTable roadmap={queryState.data!} />);

    await user.click(screen.getByRole("combobox", { name: "Subsystem" }));

    expect(await screen.findByRole("option", { name: "BSS — 2 gears" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "CORE — 1 gears" })).toBeInTheDocument();
  });

  it("shows every gear again when the choice is cleared", async () => {
    const user = userEvent.setup();
    render(<GearsTable roadmap={queryState.data!} />);

    await user.click(screen.getByRole("combobox", { name: "Subsystem" }));
    await user.click(await screen.findByRole("option", { name: /^BSS/ }));
    await user.click(screen.getByRole("combobox", { name: "Subsystem" }));
    await user.click(await screen.findByRole("option", { name: "All subsystems" }));

    expect(screen.getByText("CORE - One")).toBeInTheDocument();
  });
});

describe("sorting and drill-down", () => {
  it("asks the server for the order a header names", async () => {
    const user = userEvent.setup();
    render(<GearsTable roadmap={queryState.data!} />);

    await user.click(screen.getByRole("button", { name: /Estimate/ }));
    expect(portalRouter.search).toMatchObject({
      sort: "effort",
      dir_sort: "desc",
    });

    await user.click(screen.getByRole("button", { name: /Estimate/ }));
    expect(portalRouter.search).toMatchObject({
      sort: "effort",
      dir_sort: "asc",
    });
  });

  it("names the sorted column for a screen reader", async () => {
    const user = userEvent.setup();
    render(<GearsTable roadmap={queryState.data!} />);

    await user.click(screen.getByRole("button", { name: /Estimate/ }));

    expect(
      screen.getByRole("columnheader", { name: /Estimate/ }),
    ).toHaveAttribute("aria-sort", "descending");
  });

  it("says how many days late an overdue gear is", () => {
    queryState = {
      data: roadmap({
        gears: [
          gear({
            milestone: "2030-05",
            placement: { kind: "overdue", days: 62 },
          }),
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<GearsTable roadmap={queryState.data!} />);

    expect(screen.getByText(/2030-05 · 62d/)).toBeInTheDocument();
  });
});

describe("subsystem drill-down", () => {
  it("opens the gear list narrowed to the subsystem a summary row names", async () => {
    const user = userEvent.setup();

    render(<GearSummary roadmap={queryState.data!} />);
    await user.click(screen.getByRole("row", { name: /^CORE/ }));

    expect(portalRouter.search).toMatchObject({
      lens: "Gear list",
      subsystem: "CORE",
    });
  });

  it("reads the chosen subsystem from the URL, so a filtered list is linkable", () => {
    portalRouter.set({ subsystem: "BSS" });
    queryState = {
      data: roadmap({
        gears: [
          gear({ number: 1, subsystem: "CORE", title: "CORE - One" }),
          gear({ number: 2, subsystem: "BSS", title: "BSS - Two" }),
        ],
      }),
      isPending: false,
      isError: false,
    };

    render(<GearsTable roadmap={queryState.data!} />);

    expect(screen.getByText("BSS - Two")).toBeInTheDocument();
    expect(screen.queryByText("CORE - One")).toBeNull();
  });
});
