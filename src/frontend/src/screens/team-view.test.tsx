/**
 * "Direct reports only" scoping on the v2 team dashboard (#1724).
 *
 * The toggle (default ON) narrows the roster to depth-1 reports before it
 * reaches any query, so the member list, the heatmap, and the metric
 * collections all scope together. Covers:
 *   - default render shows direct reports only, with the scope subtitle
 *     ("Direct reports of X") and the scoped/total count on the toggle.
 *   - toggling off widens the roster to the full subtree and flips the
 *     subtitle to "X's department".
 *   - the scoped roster is what reaches the metric fetch — scoping happens
 *     upstream of it, not as a client-side row filter.
 *   - the toggle is hidden when the team has no indirect reports, where it
 *     could never change the roster (#1756).
 *
 * Child widgets are stubbed: this file tests the roster/toggle wiring, not
 * widget render rules (those have their own component tests).
 */

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { IdentityPerson, TeamMember } from "@/types/insight";

vi.mock("@/components/ic-view-toggle", () => ({
  IcViewToggle: () => null,
}));
vi.mock("@/components/ui/sidebar", () => ({
  SidebarTrigger: () => null,
}));
vi.mock("@/components/widgets/period-selector-bar", () => ({
  PeriodSelectorBar: () => null,
}));

vi.mock("@/components/widgets/dashboard/team-members-attention", () => ({
  TeamMembersAttention: () => null,
}));
vi.mock("@/components/widgets/dashboard/members-overview", () => ({
  MembersOverview: ({ members }: { members: TeamMember[] }) => (
    <div data-testid="heatmap">{members.map((m) => m.name).join(",")}</div>
  ),
}));
vi.mock("@/components/widgets/metric-views/team-metric-group-card", () => ({
  TeamMetricGroupCard: () => null,
}));
vi.mock("@/components/widgets/dashboard/group-drilldown-sheet", () => ({
  GroupDrilldownSheet: () => null,
}));

const queryState = {
  isPending: false,
  isFetching: false,
  isError: false,
  refetch: () => {},
};

const useMemberGridData = vi.fn((_collection: unknown, _entity: unknown) => ({
  byKey: new Map(),
  previousByKey: new Map(),
  ...queryState,
}));

vi.mock("@/queries/member-grid", () => ({
  useMemberGridData: (...args: [unknown, unknown]) =>
    useMemberGridData(args[0], args[1]),
}));

/** Entity ids the scoped roster sent to the heatmap fetch. */
function heatmapFetchedIds(): string[] {
  const entity = useMemberGridData.mock.lastCall?.[1] as
    | { ids: string[] }
    | undefined;
  return entity?.ids ?? [];
}

vi.mock("@/queries/metric-results", () => ({
  useMetricCollectionSet: () => new Map(),
  collectionSetPending: () => false,
}));

const PERSON_IDS = {
  alice: "019e2801-0000-7000-8000-00000000a11c",
  bob: "019e2801-0000-7000-8000-00000000b0b0",
  carol: "019e2801-0000-7000-8000-00000000ca01",
  erin: "019e2801-0000-7000-8000-00000000e21e",
  dave: "019e2801-0000-7000-8000-00000000da5e",
  grace: "019e2801-0000-7000-8000-000000009ace",
  hana: "019e2801-0000-7000-8000-00000000ba4a",
  fay: "019e2801-0000-7000-8000-00000000fa77",
  gil: "019e2801-0000-7000-8000-000000009117",
} as const;

function person(
  personId: string,
  email: string,
  name: string,
  subordinates: IdentityPerson[] = [],
): IdentityPerson {
  return {
    person_id: personId,
    email,
    display_name: name,
    subordinates,
  } as IdentityPerson;
}

// Alice manages Bob and Erin directly; Carol reports to Bob (indirect).
const viewerTree = person(PERSON_IDS.alice, "alice@x.io", "Alice", [
  person(PERSON_IDS.bob, "bob@x.io", "Bob", [
    person(PERSON_IDS.carol, "carol@x.io", "Carol"),
  ]),
  person(PERSON_IDS.erin, "erin@x.io", "Erin"),
]);

// Dave's team is flat: every report is direct, so scoping is a no-op (#1756).
const flatTree = person(PERSON_IDS.dave, "dave@x.io", "Dave", [
  person(PERSON_IDS.fay, "fay@x.io", "Fay"),
  person(PERSON_IDS.gil, "gil@x.io", "Gil"),
]);

// Grace is nobody's report in the viewer's line — she is reachable only
// through an explicit visibility grant, which identity honours and a tree walk
// never would.
const grantedTree = person(PERSON_IDS.grace, "grace@x.io", "Grace", [
  person(PERSON_IDS.hana, "hana@x.io", "Hana"),
]);

// Identity answers per person id: the screen asks for the PIVOT's profile, not
// the viewer's tree, so a request keyed by anyone else must not resolve here.
let trees: Record<string, IdentityPerson> = {};

let pivotError: unknown;
const pivotRefetch = vi.fn();

vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: (personId: string) => ({
    ...queryState,
    data: pivotError === undefined ? trees[personId] : undefined,
    error: pivotError,
    isError: pivotError !== undefined,
    refetch: pivotRefetch,
  }),
}));

import { TeamViewScreen } from "./team-view";

beforeEach(() => {
  pivotError = undefined;
  pivotRefetch.mockReset();
  trees = {
    [PERSON_IDS.alice]: viewerTree,
    [PERSON_IDS.dave]: flatTree,
    [PERSON_IDS.grace]: grantedTree,
  };
});

function renderScreen(teamId: string = PERSON_IDS.alice) {
  return render(<TeamViewScreen teamId={teamId} />);
}

describe("TeamViewScreen identity failures", () => {
  it("says the person is unavailable instead of showing an empty team", async () => {
    // A 404 pivot is not a team with no members: rendering the empty state
    // over it would report a broken lookup as an empty org.
    const { IdentityApiError } = await import("@/api/identity-client");
    pivotError = new IdentityApiError(404, {});

    renderScreen();

    expect(screen.getByText("This person is not available")).toBeInTheDocument();
    expect(screen.queryByTestId("heatmap")).not.toBeInTheDocument();
  });

  it("offers a retry when identity itself failed, not the lookup", async () => {
    const { IdentityApiError } = await import("@/api/identity-client");
    pivotError = new IdentityApiError(500, {});

    renderScreen();

    expect(screen.getByText("Unable to load this person")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(pivotRefetch).toHaveBeenCalled();
  });
});

describe("TeamViewScreen direct-reports scoping", () => {
  it("defaults to direct reports only, scoping the roster before the fetch", () => {
    renderScreen();

    expect(
      screen.getByText("Direct reports of Alice · 2 members"),
    ).toBeInTheDocument();
    expect(screen.getByText("Direct reports only")).toBeInTheDocument();
    expect(screen.getByText("(2/3)")).toBeInTheDocument();
    expect(screen.getByTestId("heatmap")).toHaveTextContent("Bob,Erin");

    expect(heatmapFetchedIds()).toEqual([PERSON_IDS.bob, PERSON_IDS.erin]);
  });

  it("widens to the whole department when toggled off", async () => {
    const user = userEvent.setup();
    renderScreen();

    await user.click(screen.getByRole("switch"));

    expect(
      screen.getByText("Alice's department · 3 members"),
    ).toBeInTheDocument();
    expect(screen.getByText("(3/3)")).toBeInTheDocument();
    expect(screen.getByTestId("heatmap")).toHaveTextContent("Bob,Carol,Erin");

    expect(heatmapFetchedIds()).toEqual([
      PERSON_IDS.bob,
      PERSON_IDS.carol,
      PERSON_IDS.erin,
    ]);
  });

  it("renders a team the viewer reaches by grant, outside their own tree", () => {
    // The pivot comes from identity, so a person absent from the viewer's
    // reporting line still gets a populated team instead of an empty one.
    renderScreen(PERSON_IDS.grace);

    expect(screen.getByText("1 member")).toBeInTheDocument();
    expect(screen.getByTestId("heatmap")).toHaveTextContent("Hana");
    expect(heatmapFetchedIds()).toEqual([PERSON_IDS.hana]);
  });

  it("hides the toggle for a team with no subteams (#1756)", () => {
    renderScreen(PERSON_IDS.dave);

    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(screen.queryByText("Direct reports only")).not.toBeInTheDocument();
    // Without the toggle the scope label is meaningless too — the subtitle
    // is just the member count, and the full roster reaches the queries.
    expect(screen.getByText("2 members")).toBeInTheDocument();
    expect(
      screen.queryByText(/Direct reports of|department/),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("heatmap")).toHaveTextContent("Fay,Gil");

    expect(heatmapFetchedIds()).toEqual([PERSON_IDS.fay, PERSON_IDS.gil]);
  });
});
