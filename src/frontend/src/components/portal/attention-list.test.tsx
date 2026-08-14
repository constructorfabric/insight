// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { portalRouter } from "@/test/portal-router";

import { act, render, renderHook, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { AttentionFlag } from "@/lib/insight/attention-flags";
import {
  usePortalZone,
} from "@/lib/portal/portal-nav";


import { pid } from "@/test/identity";

import { AttentionList } from "./attention-list";


function flag(over: Partial<AttentionFlag>): AttentionFlag {
  return {
    personId: pid("p0"),
    name: "Person",
    metricKey: "t.metric",
    metricLabel: "Commits",
    kind: "outlier",
    valueText: "2",
    reason: "well below the team median of 10",
    severity: 1,
    ...over,
  };
}

const FLAGS = Array.from({ length: 5 }, (_, i) =>
  flag({ personId: pid(`p${i}`), name: `Person ${i}`, severity: 5 - i }),
);

describe("AttentionList", () => {
  it("gives no subject more rows than the cap, however many they trip", () => {
    // A row is one finding and the list is ranked by severity, so without a
    // cap the subject in the most trouble takes the most of the visible slice
    // and pushes everyone else behind "+N more" — the list hides people better
    // the worse things are.
    const many = [
      flag({ personId: pid("p0"), name: "Busy", metricLabel: "Commits", severity: 9 }),
      flag({ personId: pid("p0"), name: "Busy", metricLabel: "PRs merged", severity: 8 }),
      flag({ personId: pid("p0"), name: "Busy", metricLabel: "Code lines", severity: 7 }),
      flag({ personId: pid("p1"), name: "Other", metricLabel: "Active days", severity: 6 }),
    ];
    render(<AttentionList flags={many} summary="s" />);
    expect(screen.getAllByText("Busy")).toHaveLength(2);
    // The one dropped is the weakest, and the other subject still shows.
    expect(screen.queryByText("Code lines")).not.toBeInTheDocument();
    expect(screen.getByText("Other")).toBeInTheDocument();
  });

  it("counts the capped rows in \"+N more\", not the raw findings", () => {
    // The toggle has to promise what expanding will actually reveal.
    const many = Array.from({ length: 4 }, (_, i) =>
      flag({
        personId: pid("p0"),
        name: "Busy",
        metricLabel: `Metric ${i}`,
        severity: 9 - i,
      }),
    );
    render(<AttentionList flags={many} summary="s" max={1} />);
    expect(screen.getByRole("button", { name: "+1 more" })).toBeInTheDocument();
  });

  it("renders the summary, people label and flag rows with reasons", () => {
    render(
      <AttentionList
        flags={FLAGS.slice(0, 2)}
        summary="2 of 8 people stand out this period — most often Commits (2)."
      />,
    );
    expect(screen.getByText(/2 of 8 people stand out this period/)).toBeInTheDocument();
    expect(screen.getByText("Person 0")).toBeInTheDocument();
    expect(screen.getAllByText("well below the team median of 10")).toHaveLength(2);
  });

  it("links every row to that person's personal page", () => {
    render(<AttentionList flags={[flag({ personId: pid("who") })]} summary="s" />);
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/ic/${pid("who")}/personal`,
    );
  });

  it("clears the pinned zone on click so the route-driven Person zone wins", async () => {
    act(() => portalRouter.set({ zone: "overview" }));
    const { result } = renderZone();
    render(<AttentionList flags={[flag({})]} summary="s" />);
    await userEvent.click(screen.getByRole("link"));
    expect(result.current).toBeNull();
  });

  it("shows the steady note when there are no flags", () => {
    render(<AttentionList flags={[]} summary="All steady." />);
    expect(screen.getByText(/No outliers, declines, or collapses/)).toBeInTheDocument();
  });

  it("collapses to max rows and expands on '+N more', then collapses back", async () => {
    render(<AttentionList flags={FLAGS} summary="s" max={2} />);
    expect(screen.getAllByRole("link")).toHaveLength(2);

    await userEvent.click(screen.getByRole("button", { name: "+3 more" }));
    expect(screen.getAllByRole("link")).toHaveLength(5);

    await userEvent.click(screen.getByRole("button", { name: "Show less" }));
    expect(screen.getAllByRole("link")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "+3 more" })).toBeInTheDocument();
  });
});

// Small helper: observe the portal zone through the public hook.
function renderZone() {
  return renderHook(() => usePortalZone());
}
