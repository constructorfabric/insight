import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import type { AttentionItem } from "@/lib/insight/attention";
import {
  metricAttentionItems,
  orderAttentionItems,
} from "@/lib/insight/attention";
import type { MetricGroup } from "@/lib/insight/groups";
import { normalizeMetricResults } from "@/lib/metrics/collection";

interface Cohort {
  p25: number;
  median: number;
  p75: number;
  min: number;
  max: number;
}

const WIDE: Cohort = { p25: 5, median: 11, p75: 15, min: 0, max: 30 };

function aiMetric(
  value: number | null,
  key = "ai.active_days",
  cohort: Cohort = WIDE
): MetricResult {
  return {
    metric_key: key,
    label: "Active AI days",
    unit: "days",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    views: [
      { view: "period", values: [{ entity_id: "me@x.com", value }] },
      {
        view: "peer",
        values: [
          {
            entity_id: "me@x.com",
            target_value: value,
            ...cohort,
            n: 9,
          },
        ],
      },
    ],
  };
}

/**
 * `ai.sessions` is the block's to show; `ai.active_days` is on the headline
 * row (it is in `KPI_ROW`) and therefore this block's to leave alone. A
 * fixture needs both to express the rule.
 */
/** What the headline row rendered — only these are the block's to skip. */
const HEADLINE = new Set(["ai.active_days"]);

const AI_DEF: MetricGroup = {
  id: "ai_adoption",
  title: "AI adoption",
  collection: {
    metrics: [
      { key: "ai.sessions", views: [{ view: "period" }, { view: "peer" }] },
      { key: "ai.active_days", views: [{ view: "period" }, { view: "peer" }] },
    ],
  },
  card: { preview: [] },
  drilldown: [],
};

function bothMetrics(value: number | null) {
  return normalizeMetricResults([
    aiMetric(value, "ai.sessions"),
    aiMetric(value, "ai.active_days"),
  ]);
}

/** A previous period the current one fell from, so a standing is also a change. */
function before(value: number) {
  return normalizeMetricResults([
    aiMetric(value, "ai.sessions"),
    aiMetric(value, "ai.active_days"),
  ]);
}

describe("metricAttentionItems", () => {
  it("surfaces bottom-quartile metrics with the same item shape", () => {
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(2),
      before(9),
      "me@x.com",
      HEADLINE
    );
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({
      key: "ai.sessions",
      group: "ai_adoption",
      valueText: "2 days",
      medianText: "11 days",
      gapText: "-82%",
    });
  });

  it("never flags unmeasured people (null peer target_value)", () => {
    const unmeasured = aiMetric(0);
    const peerView = unmeasured.views[1];
    if (peerView?.view === "peer" && peerView.values[0]) {
      peerView.values[0].target_value = null;
    }
    expect(
      metricAttentionItems(
        AI_DEF,
        normalizeMetricResults([unmeasured]),
        before(9),
        "me@x.com",
        HEADLINE
      )
    ).toHaveLength(0);
  });

  it("ignores in-pack values and missing data", () => {
    expect(
      metricAttentionItems(
        AI_DEF,
        bothMetrics(10),
        before(9),
        "me@x.com",
        HEADLINE
      )
    ).toHaveLength(0);
    expect(
      metricAttentionItems(
        AI_DEF,
        bothMetrics(null),
        before(9),
        "me@x.com",
        HEADLINE
      )
    ).toHaveLength(0);
  });
});

describe("what the headline row already shows", () => {
  it("is left to the row — the block never repeats a headline metric", () => {
    // This block is the only place on the person page that names problems, so
    // it shows everything standing out EXCEPT what the row above already
    // carries. Repeating one puts a single finding on the screen twice, and a
    // reader counts marks rather than facts.
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(2),
      before(9),
      "me@x.com",
      HEADLINE
    );
    expect(items.map((i) => i.key)).not.toContain("ai.active_days");
  });

  it("shows a metric the row does not carry, whatever the card used to list", () => {
    // `card.preview` no longer gates this: the page has no section cards, so
    // excluding those keys would hide them from the screen entirely.
    const onOldCard: MetricGroup = {
      ...AI_DEF,
      card: { preview: ["ai.sessions"] },
    };
    const items = metricAttentionItems(
      onOldCard,
      bothMetrics(2),
      before(9),
      "me@x.com",
      HEADLINE
    );
    expect(items.map((i) => i.key)).toEqual(["ai.sessions"]);
  });
});

describe("a moderate standing is not an event", () => {
  // 5 against a median of 11 is behind, but not the fraction of the cohort
  // that speaks for itself — so only a move can put it on screen.
  it("stays silent when a metric is below its cohort but did not move", () => {
    // A lead measured against the developers reporting to them is below on
    // commits every month, by the shape of the job. Repeating that forever
    // teaches the reader to skip the block; a flat gap is not news.
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(5),
      before(5),
      "me@x.com",
      HEADLINE
    );
    expect(items).toEqual([]);
  });

  it("stays silent when it moved the RIGHT way, even if still behind", () => {
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(5),
      before(4),
      "me@x.com",
      HEADLINE
    );
    expect(items).toEqual([]);
  });

  it("makes no claim about direction without a previous period", () => {
    // One period of data cannot say which way anything is going — and a
    // moderate gap has nothing else to say for itself.
    expect(
      metricAttentionItems(AI_DEF, bothMetrics(5), null, "me@x.com", HEADLINE)
    ).toEqual([]);
  });
});

describe("a state, not only a change", () => {
  it("names a metric sitting at a fraction of the cohort, unmoved", () => {
    // The worst finding a person can have was the one the block could not
    // make: fallen to almost nothing long ago and flat ever since, so no
    // period shows a change and the screen reads as "all clear".
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(1),
      before(1),
      "me@x.com",
      HEADLINE
    );
    expect(items).toHaveLength(1);
    expect(items[0]?.kind).toBe("behind");
  });

  it("names it even when there is no earlier period to compare with", () => {
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(1),
      null,
      "me@x.com",
      HEADLINE
    );
    expect(items[0]).toMatchObject({ kind: "behind", noPrevious: true });
  });

  it("calls it a fall when it also moved, the more specific claim", () => {
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(1),
      before(9),
      "me@x.com",
      HEADLINE
    );
    expect(items[0]?.kind).toBe("fell");
    expect(items[0]?.noPrevious).toBe(false);
  });
});

describe("the row's candidates are not the row", () => {
  it("shows a bottom-quartile candidate the row had no slot for", () => {
    // `KPI_ROW` lists more candidates than the row renders. Excluding the
    // whole list would hide a metric that reached neither surface — visible
    // nowhere, which is the one outcome worse than showing it twice.
    const items = metricAttentionItems(
      AI_DEF,
      bothMetrics(2),
      before(9),
      "me@x.com",
      new Set<string>() // the row rendered nothing from this group
    );
    expect(items.map((i) => i.key).sort()).toEqual([
      "ai.active_days",
      "ai.sessions",
    ]);
  });
});

describe("value split", () => {
  it("keeps a percent a percent", () => {
    // The list renders the split fields, so a lost "%" turns 50% into 50 — a
    // different number, not a shorter one.
    const pct = {
      ...aiMetric(2, "ai.sessions"),
      format: "percent" as const,
      unit: null,
    };
    const items = metricAttentionItems(
      AI_DEF,
      normalizeMetricResults([pct]),
      before(9),
      "me@x.com",
      new Set<string>()
    );
    expect(items[0]?.valueNumber).toBe("2");
    expect(items[0]?.valueUnit).toBe("%");
  });
});

describe("what counts as a move", () => {
  /** One metric, one cohort, a fall from `from` to `to`. */
  function fall(from: number, to: number, cohort: Cohort) {
    return metricAttentionItems(
      AI_DEF,
      normalizeMetricResults([aiMetric(to, "ai.sessions", cohort)]),
      normalizeMetricResults([aiMetric(from, "ai.sessions", cohort)]),
      "me@x.com",
      HEADLINE
    );
  }

  it("measures the move against the cohort, not against the person's own past", () => {
    // Halving two into one is a 50% collapse of the person's own number and
    // one event of difference. In a cohort whose quartiles are two hundred
    // apart, one event separates nobody.
    const spreadOut = { p25: 100, median: 300, p75: 500, min: 0, max: 900 };
    expect(fall(101, 100, spreadOut)).toEqual([]);
  });

  it("lets the same small move through when the cohort is that tight", () => {
    // Identical numbers, different question: here one event is a quarter of
    // what separates people, so it is a change worth naming.
    const tight = { p25: 1, median: 3, p75: 4, min: 0, max: 8 };
    expect(fall(2, 1, tight).map((i) => i.key)).toEqual(["ai.sessions"]);
  });

  it("ranks nobody in a cohort where everyone is identical", () => {
    // The quartile test alone calls someone bottom for being one of several
    // equal numbers; eligibility is what keeps that off the screen.
    const flat = { p25: 4, median: 4, p75: 4, min: 4, max: 4 };
    expect(fall(9, 4, flat)).toEqual([]);
  });
});

describe("ordering", () => {
  it("puts the metric furthest outside its cohort's spread first", () => {
    // A falls by two thirds, B by one third — so percent-of-median ranks A
    // first. But A's peers are spread across that whole distance, and B's sit
    // in a narrow band it has fallen far outside of. B is the outlier.
    const wideBand = { p25: 2, median: 3, p75: 6, min: 0, max: 9 };
    const narrowBand = { p25: 280, median: 300, p75: 320, min: 200, max: 400 };
    const items = metricAttentionItems(
      AI_DEF,
      normalizeMetricResults([
        aiMetric(1, "ai.sessions", wideBand),
        aiMetric(200, "ai.active_days", narrowBand),
      ]),
      normalizeMetricResults([
        aiMetric(3, "ai.sessions", wideBand),
        aiMetric(300, "ai.active_days", narrowBand),
      ]),
      "me@x.com",
      new Set<string>()
    );
    const byKey = new Map(items.map((i) => [i.key, i]));
    const wide = byKey.get("ai.sessions")!;
    const narrow = byKey.get("ai.active_days")!;
    expect(narrow.spreadGap).toBeGreaterThan(wide.spreadGap);
    // …and the ordering it replaces would have said the opposite.
    expect(wide.relGap).toBeGreaterThan(narrow.relGap);
  });
});

describe("a cohort that barely does the thing", () => {
  it("makes no claim about being far behind it", () => {
    // Zero against a median of one is a hundred-percent gap and no finding:
    // the metric separates nobody in this cohort, and a share of a tiny
    // number is always large, so these rows would shout the loudest.
    const scarce = { p25: 1, median: 1, p75: 2, min: 0, max: 4 };
    const items = metricAttentionItems(
      AI_DEF,
      normalizeMetricResults([aiMetric(0, "ai.sessions", scarce)]),
      normalizeMetricResults([aiMetric(0, "ai.sessions", scarce)]),
      "me@x.com",
      HEADLINE
    );
    expect(items).toEqual([]);
  });

  it("still reports a fall there, which rests on the person's own movement", () => {
    const scarce = { p25: 1, median: 1, p75: 2, min: 0, max: 4 };
    const items = metricAttentionItems(
      AI_DEF,
      normalizeMetricResults([aiMetric(0, "ai.sessions", scarce)]),
      normalizeMetricResults([aiMetric(4, "ai.sessions", scarce)]),
      "me@x.com",
      HEADLINE
    );
    expect(items[0]?.kind).toBe("fell");
  });
});

describe("orderAttentionItems", () => {
  const row = (over: Partial<AttentionItem>): AttentionItem => ({
    key: "git.commits",
    group: "git_output",
    label: "Commits",
    valueText: "1",
    valueNumber: "1",
    valueUnit: "",
    medianText: "9",
    gapText: "-89%",
    help: null,
    spreadGap: 1,
    relGap: 1,
    kind: "behind",
    noPrevious: false,
    ...over,
  });

  it("puts a standing above a fall — the larger claim first", () => {
    const ordered = orderAttentionItems(
      [row({ key: "a", kind: "fell", spreadGap: 9 }), row({ key: "b" })],
      new Set()
    );
    expect(ordered.map((i) => i.key)).toEqual(["b", "a"]);
  });

  it("keeps the stronger of two rows that restate one another", () => {
    // Ranking runs BEFORE thinning precisely so this holds: the survivor is
    // the row that says the thing more strongly, not the one that happened to
    // be evaluated first.
    const ordered = orderAttentionItems(
      [
        row({ key: "collab.meeting_hours", relGap: 0.8 }),
        row({ key: "collab.focus_time_pct", relGap: 3 }),
      ],
      new Set()
    );
    expect(ordered.map((i) => i.key)).toEqual(["collab.focus_time_pct"]);
  });

  it("says nothing the headline row already said in other units", () => {
    const ordered = orderAttentionItems(
      [row({ key: "collab.meeting_hours" })],
      new Set(["collab.focus_time_pct"])
    );
    expect(ordered).toEqual([]);
  });
});
