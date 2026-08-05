import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

// Names link to the IC page; navigation isn't exercised here, so stub Link
// to a plain anchor with the `$person` param interpolated.
vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    Link: ({
      to,
      params,
      children,
      ...rest
    }: {
      to?: string;
      params?: Record<string, string>;
      children?: React.ReactNode;
    }) => (
      <a
        href={(to ?? "").replace(
          "$person",
          encodeURIComponent(params?.person ?? ""),
        )}
        {...rest}
      >
        {children}
      </a>
    ),
  };
});

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all" }),
}));

import type { MetricResult } from "@/api/metric-results-client";
import { MembersGrid } from "@/components/widgets/dashboard/members-grid";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const MEMBERS = [
  { entityId: "ann@x.com", displayName: "Ann" },
  { entityId: "bo@x.com", displayName: "Bo" },
  { entityId: "cy@x.com", displayName: "Cy" },
];

/** One sum metric with per-member peer bands (p25 5 / median 10 / p75 15). */
function metric(
  key: string,
  perMember: Array<{
    id: string;
    value: number | null;
    suppressed?: boolean;
  }>,
  overrides: Partial<MetricResult> = {},
): MetricResult {
  return {
    metric_key: key,
    label: "Active AI days",
    short_label: "AI days",
    unit: "days",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [
      {
        view: "period",
        values: perMember.map((m) => ({ entity_id: m.id, value: m.value })),
      },
      {
        view: "peer",
        values: perMember.map((m) => ({
          entity_id: m.id,
          target_value: m.value,
          p25: m.suppressed ? null : 5,
          median: m.suppressed ? null : 10,
          p75: m.suppressed ? null : 15,
          min: m.suppressed ? null : 0,
          max: m.suppressed ? null : 30,
          n: m.suppressed ? 3 : 12,
        })),
      },
    ],
    ...overrides,
  } as MetricResult;
}

function byKeyFor(...metrics: MetricResult[]) {
  return normalizeMetricResults(metrics);
}

describe("MembersGrid", () => {
  it("renders a semantic table: sortable metric columns, member row headers linking to the IC view", () => {
    render(
      <MembersGrid
        members={MEMBERS}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [
            { id: "ann@x.com", value: 20 },
            { id: "bo@x.com", value: 8 },
            { id: "cy@x.com", value: 2 },
          ]),
        )}
        caption="Members grid"
      />,
    );
    const table = screen.getByRole("table", { name: "Members grid" });
    // Header carries the server short label; the sort control names the full one.
    expect(
      within(table).getByRole("button", {
        name: "Active AI days — sort by this column",
      }),
    ).toHaveTextContent("AI days");
    const rowHeaders = within(table).getAllByRole("rowheader");
    expect(rowHeaders).toHaveLength(3);
    const annLink = within(table).getByRole("link", { name: "Ann" });
    expect(annLink).toHaveAttribute(
      "href",
      "/ic/ann%40x.com/personal",
    );
    // No issues facet → the Member header is a plain name toggle, no menu,
    // and no issues chip.
    expect(
      within(table).getByRole("button", { name: "Member" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Most behind")).not.toBeInTheDocument();
    expect(screen.queryByText(/on par/)).not.toBeInTheDocument();
  });

  it("skips metric keys absent from the results", () => {
    render(
      <MembersGrid
        members={MEMBERS}
        metricKeys={["ai.active_days", "ai.missing"]}
        byKey={byKeyFor(
          metric("ai.active_days", [{ id: "ann@x.com", value: 20 }]),
        )}
        caption="Members grid"
      />,
    );
    // Member column + one metric column only.
    expect(screen.getAllByRole("columnheader")).toHaveLength(2);
  });

  it("sorts best-first on column click, flips on the second click, missing always last", async () => {
    const user = userEvent.setup();
    render(
      <MembersGrid
        members={MEMBERS}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [
            { id: "ann@x.com", value: 8 },
            { id: "bo@x.com", value: 20 },
            { id: "cy@x.com", value: null },
          ]),
        )}
        caption="Members grid"
      />,
    );
    const names = () =>
      screen.getAllByRole("rowheader").map((th) => th.textContent);
    // No facet and no cell trails its cohort → issues all 0, tiebroken by name.
    expect(names()).toEqual(["Ann", "Bo", "Cy"]);

    const sortButton = screen.getByRole("button", {
      name: "Active AI days — sort by this column",
    });
    await user.click(sortButton);
    // higher_is_better best-first: Bo (20), Ann (8); unmeasured Cy last.
    expect(names()).toEqual(["Bo", "Ann", "Cy"]);
    expect(
      screen.getAllByRole("columnheader")[1],
    ).toHaveAttribute("aria-sort", "descending");

    await user.click(sortButton);
    // Flipped: worst-first, but missing still sorts last.
    expect(names()).toEqual(["Ann", "Bo", "Cy"]);
    expect(
      screen.getAllByRole("columnheader")[1],
    ).toHaveAttribute("aria-sort", "ascending");
  });

  it("renders — for unobserved members and neutral for suppressed cohorts", () => {
    render(
      <MembersGrid
        members={MEMBERS.slice(0, 2)}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [
            { id: "ann@x.com", value: null },
            { id: "bo@x.com", value: 4, suppressed: true },
          ]),
        )}
        caption="Members grid"
      />,
    );
    expect(
      screen.getByRole("button", {
        name: /Ann — Active AI days: not recorded/,
      }),
    ).toBeInTheDocument();
    // Suppressed cohort (n < 5): the member's value shows without a standing.
    expect(
      screen.getByRole("button", {
        name: /Bo — Active AI days: 4 days — No peer data/,
      }),
    ).toBeInTheDocument();
  });

  it("shows the standing facet: chip, worst line, and behind-first default sort", () => {
    render(
      <MembersGrid
        members={MEMBERS.slice(0, 2)}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [
            { id: "ann@x.com", value: 20 },
            { id: "bo@x.com", value: 2 },
          ]),
        )}
        countsByMember={
          new Map([
            ["ann@x.com", { top: 2, inPack: 1, bottom: 0, unranked: 0 }],
            ["bo@x.com", { top: 0, inPack: 0, bottom: 3, unranked: 0 }],
          ])
        }
        worstByMember={new Map([["bo@x.com", "Time to resolution"]])}
        caption="Members grid"
      />,
    );
    // Bo (3 behind) leads under the default behind-first sort; Ann's chip
    // surfaces her strengths instead of a flat "on par".
    const rowHeaders = screen.getAllByRole("rowheader");
    expect(rowHeaders[0]).toHaveTextContent("Bo");
    expect(rowHeaders[0]).toHaveTextContent("3 behind peers");
    expect(rowHeaders[0]).toHaveTextContent("worst: Time to resolution");
    expect(rowHeaders[1]).toHaveTextContent("2 ahead of peers");
    // With the facet, the Member header is a menu (the roster-ordering control).
    expect(
      screen.getByRole("button", { name: /Member/ }),
    ).toBeInTheDocument();
  });

  it("derives a group-local standing facet from its own cells when showIssues is set", () => {
    render(
      <MembersGrid
        members={MEMBERS}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [
            { id: "ann@x.com", value: 2 }, // below p25 → trails
            { id: "bo@x.com", value: 20 }, // above p75 → ahead
            { id: "cy@x.com", value: 8 }, // in pack
          ]),
        )}
        showIssues
        caption="Members grid"
      />,
    );
    const rowHeaders = screen.getAllByRole("rowheader");
    // Ann trails this group's metric → behind; Bo leads it → ahead; Cy is
    // genuinely mid-pack → on par.
    expect(rowHeaders[0]).toHaveTextContent("Ann");
    expect(rowHeaders[0]).toHaveTextContent("1 behind peers");
    expect(rowHeaders[0]).toHaveTextContent("worst: Active AI days");
    expect(rowHeaders[1]).toHaveTextContent("Bo");
    expect(rowHeaders[1]).toHaveTextContent("1 ahead of peers");
    expect(rowHeaders[2]).toHaveTextContent("Cy");
    expect(rowHeaders[2]).toHaveTextContent("on par with peers");
  });

  it("defaults to most-trailing-first from its own cells when no facet is given", () => {
    render(
      <MembersGrid
        members={MEMBERS}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [
            { id: "ann@x.com", value: 20 }, // top
            { id: "bo@x.com", value: 8 }, // in pack
            { id: "cy@x.com", value: 2 }, // bottom
          ]),
        )}
        caption="Members grid"
      />,
    );
    const names = screen
      .getAllByRole("rowheader")
      .map((th) => th.textContent);
    expect(names).toEqual(["Cy", "Ann", "Bo"]);
  });

  it("shows a trend arrow against the previous period", () => {
    render(
      <MembersGrid
        members={[MEMBERS[0]!]}
        metricKeys={["ai.active_days"]}
        byKey={byKeyFor(
          metric("ai.active_days", [{ id: "ann@x.com", value: 20 }]),
        )}
        previousByKey={byKeyFor(
          metric("ai.active_days", [{ id: "ann@x.com", value: 10 }]),
        )}
        caption="Members grid"
      />,
    );
    const cell = screen.getByRole("button", {
      name: /Ann — Active AI days: 20 days/,
    });
    expect(cell.querySelector("svg")).toBeInTheDocument();
  });
});
