import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all" }),
}));

// The grid and triage list have their own component tests; here we assert the
// overview's mapping (roster → grid members, cross-group counts, worst label,
// triage rows) by capturing the props it computes.
const gridProps = vi.fn();
const triageProps = vi.fn();
vi.mock("@/components/widgets/dashboard/members-grid", () => ({
  MembersGrid: (props: unknown) => {
    gridProps(props);
    return <div data-testid="grid" />;
  },
}));
vi.mock("@/components/widgets/dashboard/triage-list", () => ({
  TriageList: (props: unknown) => {
    triageProps(props);
    return <div data-testid="triage" />;
  },
}));

import { MembersOverview } from "@/components/widgets/dashboard/members-overview";
import { HEATMAP_METRIC_KEYS } from "@/lib/insight/groups";
import type { PeerStoryEntry } from "@/lib/metrics/peer-story";
import type { TeamMember } from "@/types/insight";

function member(person_id: string, name: string): TeamMember {
  return {
    person_id,
    period: "month",
    name,
    seniority: "",
    supervisor_email: null,
    org_unit_id: null,
    tasks_closed: 0,
    bugs_fixed: 0,
    dev_time_h: null,
    prs_merged: null,
    build_success_pct: null,
    focus_time_pct: null,
    ai_tools: [],
    ai_loc_share_pct: null,
  } as TeamMember;
}

function entry(
  status: PeerStoryEntry["status"],
  label: string,
): PeerStoryEntry {
  return {
    key: label,
    label,
    status,
    format: "integer",
    unit: null,
    stats: { p25: 1, p50: 2, p75: 3, min: 0, max: 4, n: 8 },
    gapPct: 0.5,
    gapDelta: 1,
  } as PeerStoryEntry;
}

describe("MembersOverview", () => {
  it("maps the roster into grid members, cross-group counts, and a worst label", () => {
    gridProps.mockReset();
    render(
      <MembersOverview
        members={[member("a@x.com", "Ann"), member("b@x.com", "Bo")]}
        heatmapByKey={new Map()}
        previousHeatmapByKey={new Map()}
        metricBelowByMember={new Map([["a@x.com", 2]])}
        metricEntriesByPerson={
          new Map([
            ["a@x.com", [entry("bottom", "Resolution"), entry("top", "Commits")]],
            ["b@x.com", [entry("top", "Commits")]],
          ])
        }
      />,
    );

    expect(screen.getByText("Members × metrics")).toBeInTheDocument();
    const props = gridProps.mock.calls.at(-1)![0] as {
      members: { entityId: string; displayName: string }[];
      metricKeys: readonly string[];
      countsByMember: Map<string, { top: number; bottom: number }>;
      worstByMember: Map<string, string | null>;
    };
    expect(props.members.map((m) => m.displayName)).toEqual(["Ann", "Bo"]);
    expect(props.metricKeys).toEqual(HEATMAP_METRIC_KEYS);
    expect(props.countsByMember.get("a@x.com")).toMatchObject({
      top: 1,
      bottom: 1,
    });
    // Worst = the most-severe bottom entry across all groups.
    expect(props.worstByMember.get("a@x.com")).toBe("Resolution");
    expect(props.worstByMember.get("b@x.com")).toBeNull();
  });

  it("feeds the mobile triage list the below/top counts per member", () => {
    triageProps.mockReset();
    render(
      <MembersOverview
        members={[member("a@x.com", "Ann")]}
        heatmapByKey={new Map()}
        previousHeatmapByKey={new Map()}
        metricBelowByMember={new Map([["a@x.com", 3]])}
        metricEntriesByPerson={
          new Map([["a@x.com", [entry("top", "Commits")]]])
        }
      />,
    );
    const props = triageProps.mock.calls.at(-1)![0] as {
      rows: { belowCount: number; topCount: number }[];
    };
    expect(props.rows[0]).toMatchObject({ belowCount: 3, topCount: 1 });
  });
});
