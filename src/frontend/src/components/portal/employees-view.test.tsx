// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
/**
 * Employees directory semantics: the org tree flattens into a de-duplicated,
 * sorted roster; search filters across name/title/department; rows link into
 * Person; identity failures surface as a retryable error.
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { identityPerson, pid } from "@/test/identity";
import type { IdentityPerson } from "@/types/insight";

const mocks = vi.hoisted(() => ({
  personId: null as string | null,
  isFlat: false,
  roster: {
    roster: [] as { person_id: string; display_name?: string | null; username?: string | null; job_title?: string | null; status?: string | null; provisional?: boolean }[],
    truncated: false,
    isPending: false,
    isError: false,
    retry: vi.fn(),
  },
  ic: {
    data: undefined as IdentityPerson | undefined,
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  },
}));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "boss@x", personId: mocks.personId }),
}));
vi.mock("@/queries/ic-dashboard", () => ({ useIcPerson: () => mocks.ic }));
vi.mock("@/queries/identity-me", () => ({
  useVisibilityPolicy: () => ({
    policy: mocks.isFlat ? "flat" : "org_chart",
    isFlat: mocks.isFlat,
    isPending: false,
  }),
}));
vi.mock("@/queries/visible-roster", () => ({
  useVisibleRoster: () => mocks.roster,
}));

import { EmployeesView } from "./employees-view";

const person = (
  label: string,
  over: Partial<IdentityPerson> = {},
  subs: IdentityPerson[] = [],
): IdentityPerson => identityPerson(label, over, subs);

beforeEach(() => {
  mocks.ic.refetch.mockClear();
  mocks.personId = pid("boss");
  mocks.isFlat = false;
  mocks.roster.roster = [];
  mocks.roster.isPending = false;
  mocks.roster.isError = false;
  mocks.ic.isPending = false;
  mocks.ic.isError = false;
  mocks.ic.data = person("boss", { display_name: "Boss", job_title: "Director" }, [
    person("zoe", { display_name: "Zoe", job_title: "QA Engineer", department: "Quality" } as never),
    person("adam", { display_name: "Adam", job_title: "Backend Dev" } as never, [
      // the same person deeper in the tree must NOT double a row
      person("zoe", { display_name: "Zoe" } as never),
    ]),
  ]);
});

describe("EmployeesView", () => {
  it("flattens the tree, de-duplicates and sorts by display name", () => {
    render(<EmployeesView />);
    const rows = screen.getAllByRole("link").map((a) => a.textContent);
    expect(rows).toEqual(["Adam", "Boss", "Zoe"]);
    expect(screen.getByText(/3 people/)).toBeInTheDocument();
  });

  it("links each row into that person's Person view", () => {
    render(<EmployeesView />);
    expect(screen.getAllByRole("link")[2]).toHaveAttribute(
      "href",
      `/ic/${pid("zoe")}/personal`,
    );
  });

  it("filters across name / title / department and shows the filtered count", async () => {
    render(<EmployeesView />);
    await userEvent.type(screen.getByRole("textbox"), "quality");
    expect(screen.getAllByRole("link").map((a) => a.textContent)).toEqual(["Zoe"]);
    expect(screen.getByText(/1 of 3/)).toBeInTheDocument();
  });

  it("surfaces an identity failure as a retryable error", async () => {
    mocks.ic.data = undefined;
    mocks.ic.isError = true;
    render(<EmployeesView />);
    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(mocks.ic.refetch).toHaveBeenCalledOnce();
  });
});

describe("EmployeesView on an organisation with no reporting lines", () => {
  beforeEach(() => {
    mocks.isFlat = true;
    // The tree answers with the viewer alone — the case that used to leave this
    // directory with a single row.
    mocks.ic.data = person("boss");
    mocks.roster.roster = [
      { person_id: pid("boss"), display_name: "Boss Person", job_title: "Lead" },
      { person_id: pid("ann"), display_name: "Ann Dev", job_title: "Engineer" },
      { person_id: pid("bob"), username: "bobby", provisional: true },
    ];
  });

  it("is headed as a roster, not as employees", () => {
    render(<EmployeesView />);

    expect(
      screen.getByRole("heading", { level: 1, name: "Roster" }),
    ).toBeInTheDocument();
  });

  it("lists the roster rather than the tree", () => {
    render(<EmployeesView />);

    expect(screen.getByText("Ann Dev")).toBeInTheDocument();
    expect(screen.getByText("Boss Person")).toBeInTheDocument();
  });

  it("shows a person the journal knows only by a handle under that handle", () => {
    render(<EmployeesView />);

    expect(screen.getByText("bobby")).toBeInTheDocument();
  });

  it("drops the columns a flat organisation cannot fill", () => {
    render(<EmployeesView />);

    expect(screen.queryByRole("columnheader", { name: "Manager" })).toBeNull();
    expect(screen.queryByRole("columnheader", { name: "Department" })).toBeNull();
    expect(screen.getByRole("columnheader", { name: "Name" })).toBeInTheDocument();
  });

  it("filters the roster by the term typed", async () => {
    render(<EmployeesView />);

    await userEvent.type(screen.getByPlaceholderText(/Search/), "ann");

    expect(screen.getByText("Ann Dev")).toBeInTheDocument();
    expect(screen.queryByText("Boss Person")).toBeNull();
  });

  it("offers a retry when the roster read fails, rather than an empty directory", () => {
    mocks.roster.roster = [];
    mocks.roster.isError = true;

    render(<EmployeesView />);

    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});
