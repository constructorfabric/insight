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
import { usePortalZone } from "@/lib/portal/portal-nav";

import { pid } from "@/test/identity";

import { AttentionList } from "./attention-list";

function flag(over: Partial<AttentionFlag>): AttentionFlag {
  return {
    personId: pid("p0"),
    name: "Person",
    metricKey: "t.commits",
    metricLabel: "Commits",
    kind: "outlier",
    moved: "down",
    valueText: "2",
    reason: "well below the team median of 10",
    severity: 1,
    ...over,
  };
}

/** Three people on Commits, one on Meetings. */
const MIXED = [
  ...Array.from({ length: 3 }, (_, i) =>
    flag({ personId: pid(`p${i}`), name: `Person ${i}`, severity: 3 - i })
  ),
  flag({
    personId: pid("p0"),
    name: "Person 0",
    metricKey: "t.meetings",
    metricLabel: "Meeting hours",
    severity: 9,
  }),
];

async function openTheme(name: RegExp) {
  await userEvent.click(screen.getByRole("button", { name }));
}

describe("AttentionList", () => {
  it("leads with the metric and how many people it is about", () => {
    render(<AttentionList flags={MIXED} summary="s" />);
    expect(
      screen.getByRole("button", { name: /Commits\s*3 people/ })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Meeting hours\s*1 person/ })
    ).toBeInTheDocument();
  });

  it("puts the metric that touches most people first", () => {
    render(<AttentionList flags={MIXED} summary="s" />);
    const headers = screen.getAllByRole("button").map((b) => b.textContent);
    expect(headers[0]).toMatch(/Commits/);
    expect(headers[1]).toMatch(/Meeting hours/);
  });

  it("names nobody until a metric is opened", () => {
    render(<AttentionList flags={MIXED} summary="s" />);
    expect(screen.queryByText("Person 1")).not.toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("opens a metric into its people, strongest first", async () => {
    render(<AttentionList flags={MIXED} summary="s" />);
    await openTheme(/Commits/);
    const names = screen
      .getAllByRole("link")
      .map((a) => a.textContent?.match(/Person \d/)?.[0]);
    expect(names).toEqual(["Person 0", "Person 1", "Person 2"]);
  });

  it("lists one person under each metric they trip", async () => {
    render(<AttentionList flags={MIXED} summary="s" />);
    await openTheme(/Commits/);
    await openTheme(/Meeting hours/);
    expect(screen.getAllByText("Person 0")).toHaveLength(2);
  });

  it("links a person to their own page", async () => {
    render(
      <AttentionList flags={[flag({ personId: pid("who") })]} summary="s" />
    );
    await openTheme(/Commits/);
    expect(screen.getByRole("link")).toHaveAttribute(
      "href",
      `/ic/${pid("who")}/personal`
    );
  });

  it("clears the pinned zone on click so the route-driven Person zone wins", async () => {
    act(() => portalRouter.set({ zone: "overview" }));
    const { result } = renderHook(() => usePortalZone());
    render(<AttentionList flags={[flag({})]} summary="s" />);
    await openTheme(/Commits/);
    await userEvent.click(screen.getByRole("link"));
    expect(result.current).toBeNull();
  });

  it("caps the metrics listed, and expands on request", async () => {
    render(<AttentionList flags={MIXED} summary="s" max={1} />);
    expect(screen.queryByRole("button", { name: /Meeting hours/ })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "+1 more" }));
    expect(
      screen.getByRole("button", { name: /Meeting hours/ }),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.queryByRole("button", { name: /Meeting hours/ })).not.toBeInTheDocument();
  });

  it("caps the people inside one metric, and expands on request", async () => {
    // 14 people on Commits: the twelfth is the last one listed.
    const many = Array.from({ length: 14 }, (_, i) =>
      flag({ personId: pid(`q${i}`), name: `Q${i}`, severity: 14 - i }),
    );
    render(<AttentionList flags={many} summary="s" />);
    await openTheme(/Commits/);
    expect(screen.getAllByRole("link")).toHaveLength(12);

    await userEvent.click(screen.getByRole("button", { name: "+2 more" }));
    expect(screen.getAllByRole("link")).toHaveLength(14);
  });

  it("points the arrow the way the number actually went", async () => {
    // A rise is adverse on a lower-is-better metric. A down arrow beside
    // "well above the median" contradicts the sentence next to it.
    render(
      <AttentionList
        flags={[
          flag({
            metricLabel: "Meeting hours",
            metricKey: "t.meetings",
            moved: "up",
            reason: "well above the team median of 5 h",
          }),
        ]}
        summary="s"
      />,
    );
    await openTheme(/Meeting hours/);
    const link = screen.getByRole("link");
    expect(link.querySelector(".lucide-arrow-up-right")).not.toBeNull();
    expect(link.querySelector(".lucide-arrow-down-right")).toBeNull();
  });

  it("says so when nothing stands out", () => {
    render(<AttentionList flags={[]} summary="All steady." />);
    expect(
      screen.getByText("Nothing stands out this period.")
    ).toBeInTheDocument();
  });
});
