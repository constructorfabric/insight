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
    reason: "unusually low · team median 10",
    severity: 1,
    ...over,
  };
}

const FLAGS = Array.from({ length: 5 }, (_, i) =>
  flag({ personId: pid(`p${i}`), name: `Person ${i}`, severity: 5 - i }),
);

describe("AttentionList", () => {
  it("renders the summary, people label and flag rows with reasons", () => {
    render(
      <AttentionList
        flags={FLAGS.slice(0, 2)}
        summary="2 of 8 people need a look — most flags on Commits (2)."
        peopleLabel="2 of 8 people"
      />,
    );
    expect(screen.getByText(/2 of 8 people need a look/)).toBeInTheDocument();
    expect(screen.getByText("Person 0")).toBeInTheDocument();
    expect(screen.getAllByText("unusually low · team median 10")).toHaveLength(2);
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
