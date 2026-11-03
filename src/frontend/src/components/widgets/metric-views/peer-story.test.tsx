import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { EvidenceDialogContext } from "@/components/metric-evidence-context";
import { PeerStory } from "@/components/widgets/metric-views/peer-story";
import {
  buildPeerStoryEntries,
  type PeerStoryEntry,
} from "@/lib/metrics/peer-story";
import {
  normalizeMetricResults,
  type MetricCollectionConfig,
} from "@/lib/metrics/collection";
import type { MetricResult } from "@/api/metric-results-client";

// value relative to a fixed cohort (p25 5, median 10, p75 15) decides status:
// >=15 top, <=5 bottom, else in_pack.
function metric(key: string, value: number): MetricResult {
  return {
    metric_key: key,
    label: key,
    unit: "days",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    drilldown: { granularity: ["event"] },
    selection: {
      metric_key: key,
      entity: { type: "person", ids: ["me@x.com"] },
      period: { from: "2026-07-01", to: "2026-07-31" },
      filters: [],
    },
    views: [
      { view: "period", values: [{ entity_id: "me@x.com", value }] },
      {
        view: "peer",
        values: [
          {
            entity_id: "me@x.com",
            target_value: value,
            p25: 5,
            median: 10,
            p75: 15,
            min: 0,
            max: 30,
            n: 12,
          },
        ],
      },
    ],
  };
}

function entriesFrom(spec: Array<[string, number]>): PeerStoryEntry[] {
  const collection: MetricCollectionConfig = {
    metrics: spec.map(([key]) => ({
      key,
      views: [{ view: "period" }, { view: "peer" }],
    })),
  };
  const byKey = normalizeMetricResults(spec.map(([key, v]) => metric(key, v)));
  return buildPeerStoryEntries(collection, byKey, "me@x.com");
}

describe("PeerStory", () => {
  it("renders the empty card when there are no entries", () => {
    render(<PeerStory entries={[]} emptyLabel="Nothing here yet." />);
    expect(screen.getByText("Nothing here yet.")).toBeInTheDocument();
  });

  it("promotes the bottom outlier to the hero and folds on-par metrics", () => {
    render(
      <PeerStory
        entries={entriesFrom([
          ["issue", 1],
          ["win", 30],
          ["par", 10],
        ])}
      />
    );
    expect(screen.getByText("Furthest behind")).toBeInTheDocument();
    // The in-pack metric lands in the supporting fold toggle.
    expect(screen.getByText(/near the median/i)).toBeInTheDocument();
  });

  it("promotes the bottom outlier over the top one", () => {
    render(
      <PeerStory
        entries={entriesFrom([
          ["issue", 1],
          ["win", 30],
        ])}
      />
    );
    expect(screen.getByText("Furthest behind")).toBeInTheDocument();
    expect(screen.queryByText("Furthest ahead")).not.toBeInTheDocument();
    expect(screen.getByText("win")).toBeInTheDocument();
  });

  it("promotes the top outlier when nobody is behind", () => {
    render(
      <PeerStory
        entries={entriesFrom([
          ["win", 30],
          ["par", 10],
        ])}
      />
    );
    expect(screen.getByText("Furthest ahead")).toBeInTheDocument();
  });

  it("lays a profile with no outliers out flat, with supporting-data actions", async () => {
    const user = userEvent.setup();
    const openEvidenceTargets = vi.fn();
    render(
      <EvidenceDialogContext.Provider
        value={{
        openEvidence: vi.fn(),
        openEvidenceTargets,
        openEvidencePeople: vi.fn(),
      }}
      >
        <PeerStory
          entries={entriesFrom([
            ["issue", 8],
            ["win", 12],
          ])}
        />
      </EvidenceDialogContext.Provider>
    );
    expect(screen.queryByText("Furthest behind")).not.toBeInTheDocument();
    expect(screen.queryByText("Furthest ahead")).not.toBeInTheDocument();
    expect(screen.getByText("issue")).toBeInTheDocument();
    expect(screen.getByText("win")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "More actions for issue" })
    );
    await user.click(
      await screen.findByRole("menuitem", { name: "View supporting data" })
    );
    expect(openEvidenceTargets).toHaveBeenCalledWith(
      [
        {
          selection: expect.objectContaining({ metric_key: "issue" }),
          label: "issue",
        },
      ],
      { activeMetricKey: "issue" }
    );
  });

  it("shows a multiple for a gap at/above 2× the median", () => {
    // 30 vs median 10 = 3× → "3×", not the exploding "+200%".
    render(<PeerStory entries={entriesFrom([["win", 30]])} />);
    expect(screen.getByText("3×")).toBeInTheDocument();
    expect(screen.queryByText("+200%")).not.toBeInTheDocument();
  });

  it("keeps a signed percent for a gap under 2× the median", () => {
    // 15 vs median 10 = 1.5× (< 2×) → "+50%".
    render(<PeerStory entries={entriesFrom([["win", 15]])} />);
    expect(screen.getByText("+50%")).toBeInTheDocument();
  });

  it("overflow outliers beyond the hero and side cards become chips", () => {
    render(
      <PeerStory
        entries={entriesFrom([
          ["i1", 1],
          ["i2", 1],
          ["i3", 1],
          ["i4", 1],
          ["i5", 1],
          ["i6", 1],
        ])}
      />
    );
    // hero (1) + side cards (3) leaves 2 as chips; the 6th label renders once.
    expect(screen.getByText("i6")).toBeInTheDocument();
  });
});
