const standings = vi.hoisted(() => ({ list: [] as unknown[] }));
vi.mock("@/lib/portal/use-person-sections", () => ({
  usePersonSectionStandings: () => standings.list,
}));

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { OwnCoverage } from "./own-coverage";

function standing(
  id: string,
  title: string,
  overrides: Partial<{
    hasData: boolean;
    peersHaveData: boolean;
    isPending: boolean;
    isError: boolean;
  }> = {}
) {
  return {
    id,
    title,
    status: "neutral",
    phrase: "",
    hasData: true,
    peersHaveData: true,
    isPending: false,
    isError: false,
    refetch: vi.fn(),
    ...overrides,
  };
}

beforeEach(() => {
  standings.list = [];
});

describe("<OwnCoverage>", () => {
  it("says for each section whether it has data, was quiet, or has no source", () => {
    standings.list = [
      standing("git_output", "Git output"),
      standing("collab", "Collaboration", { hasData: false }),
      standing("wiki", "Wiki", { hasData: false, peersHaveData: false }),
    ];

    render(<OwnCoverage personId="me-1" onOpenSection={vi.fn()} />);

    expect(
      screen.getAllByRole("button").map((row) => row.getAttribute("aria-label"))
    ).toEqual([
      "Git output: Has data",
      "Collaboration: Nothing recorded this period",
      "Wiki: No data source connected",
    ]);
  });

  it("opens the section a row names", async () => {
    const onOpenSection = vi.fn();
    standings.list = [standing("git_output", "Git output")];

    render(<OwnCoverage personId="me-1" onOpenSection={onOpenSection} />);
    await userEvent.click(screen.getByRole("button", { name: /Git output/ }));

    expect(onOpenSection).toHaveBeenCalledWith("git_output");
  });

  it("waits while any section is still loading", () => {
    standings.list = [
      standing("git_output", "Git output"),
      standing("wiki", "Wiki", { isPending: true }),
    ];

    render(<OwnCoverage personId="me-1" onOpenSection={vi.fn()} />);

    expect(screen.queryByRole("list", { name: "Sections" })).toBeNull();
  });

  it("offers a retry rather than a verdict when a request failed", () => {
    standings.list = [
      standing("wiki", "Wiki", { hasData: false, isError: true }),
    ];

    render(<OwnCoverage personId="me-1" onOpenSection={vi.fn()} />);

    expect(screen.queryByText("Nothing recorded this period")).toBeNull();
    expect(
      screen.getByRole("button", { name: /retry|try again/i })
    ).toBeInTheDocument();
  });
});
