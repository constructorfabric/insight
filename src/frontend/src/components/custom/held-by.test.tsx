vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Holder } from "@/api/custom-client";

import { Held } from "./held-by";

function card(holders: Holder[]) {
  return render(
    <Held
      title="Used by"
      empty="Nothing draws it yet."
      pending={false}
      error={null}
      holders={holders}
    />
  );
}

describe("<Held>", () => {
  it("names what holds it, and says nothing more while it all still works", () => {
    card([
      { kind: "widgets", name: "commits_table" },
      { kind: "widgets", name: "commits_line" },
    ]);

    expect(screen.getByText("commits_table")).toBeInTheDocument();
    expect(screen.getByText("commits_line")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  // A metric can be changed out from under the widgets that draw it, so the
  // reader who just changed it is told by name rather than finding an empty
  // card on a board later.
  it("says which one no longer draws what the definition produces", () => {
    card([
      {
        kind: "widgets",
        name: "commits_table",
        broken: "`total` is not a column of metric `commits_per_day`",
      },
      { kind: "widgets", name: "commits_line" },
    ]);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "commits_table no longer draws what this produces."
    );
    expect(
      screen.getByText(/`total` is not a column of metric/)
    ).toBeInTheDocument();
  });

  it("counts them and names each when several are broken", () => {
    card([
      { kind: "widgets", name: "one", broken: "gone" },
      { kind: "widgets", name: "two", broken: "gone" },
      { kind: "widgets", name: "three" },
    ]);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "2 of these no longer draw what this produces: one, two."
    );
  });

  it("says there are none rather than showing an empty list", () => {
    card([]);

    expect(screen.getByText("Nothing draws it yet.")).toBeInTheDocument();
  });
});
